use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering as AOrd};
use std::sync::{Arc, Mutex};
use std::marker::Send;
use rand::Rng;
use rand;
use std::ptr::NonNull;
use std::fmt;

use wtree::WeightTree;
use timer::{Timer, Runer};
use fast_deque::FastDeque;

#[derive(Debug)]
pub enum TaskPoolErr{
    NoSyncQueue(Arc<AtomicUsize>), //同步队列不存在
    SyncQueueLockExist(Arc<AtomicUsize>), //同步队列锁已存在
}

pub struct TaskPool<T: 'static>{
    sync_pool: Arc<(AtomicUsize, Mutex<SyncPool<T>>)>,
    async_pool: Arc<(AtomicUsize, Mutex<AsyncPool<T>>)>,
    delay_queue: Timer<DelayTask<T>>,
}

impl<T: 'static> TaskPool<T>{
    pub fn new(timer: Timer<DelayTask<T>>,) -> Self {
        // let timer = Timer::new(10);
        // timer.run();
        TaskPool {
            sync_pool: Arc::new((AtomicUsize::new(0), Mutex::new(SyncPool::new()))),
            async_pool: Arc::new((AtomicUsize::new(0), Mutex::new(AsyncPool::new()))),
            delay_queue: timer,
        }
    }
    
    /// create sync queues, return true, or false if id is exist
    pub fn create_sync_queue(&self, weight: usize, can_del: bool) -> Arc<AtomicUsize> {
        self.sync_pool.1.lock().unwrap().create_queue(weight, can_del)
    }

    /// delete sync queues, return true, or false if id is not exist
    pub fn delete_sync_queue(&self, id: &Arc<AtomicUsize>) -> bool {
        self.sync_pool.1.lock().unwrap().try_remove(id)
    }

    /// push a sync task, return Ok(index), or Err if queue id is exist
    pub fn push_sync(&self, task: T, id: &Arc<AtomicUsize>, direc: Direction) -> Result<Option<Arc<AtomicUsize>>, TaskPoolErr>{
        let mut sync_pool = self.sync_pool.1.lock().unwrap();
        match direc {
            Direction::Front => {
                let r = sync_pool.push_front(id, task);
                self.sync_pool.0.store(sync_pool.get_weight(), AOrd::Relaxed);
                r
            },
            Direction::Back => {
                let r = sync_pool.push_back(id, task);
                self.sync_pool.0.store(sync_pool.get_weight(), AOrd::Relaxed);
                r
            }
        }
    }

    /// push a async task
    pub fn push_async(&self, task: T, priority: usize){
        let mut lock = self.async_pool.1.lock().unwrap();
        lock.push(task, priority);
        self.async_pool.0.store(lock.amount(), AOrd::Relaxed);
    }

    /// push a async task, return Arc<AtomicUsize> as index, the task can removed with index
    pub fn push_async_with_index(&self, task: T, priority: usize) -> Arc<AtomicUsize> {
        let mut lock = self.async_pool.1.lock().unwrap();
        let r = lock.push(task, priority);
        self.async_pool.0.store(lock.amount(), AOrd::Relaxed);
        r
    }

    /// push a delay task, return Arc<AtomicUsize> as index
    pub fn push_delay(&self, task: T, task_type: TaskType, ms: u32) -> Arc<AtomicUsize> {
        let r = match task_type {
            TaskType::Async(priority) => {
                DelayTask::Async {
                    priority: priority,
                    async_pool: self.async_pool.clone(),
                    task: Box::into_raw_non_null(Box::new(task)),
                }
            },
            TaskType::Sync(id, direc) => {
                DelayTask::Sync {
                    id: id,
                    direc: direc,
                    sync_pool: self.sync_pool.clone(),
                    task: Box::into_raw_non_null(Box::new(task)),
                }
            },
        };
        self.delay_queue.set_timeout(r, ms)
    }

    /// pop a task
    pub fn pop_unlock(&self) -> Option<T>{
        let async_w = self.async_pool.0.load(AOrd::Relaxed);  //异步池总权重
        let sync_w = self.sync_pool.0.load(AOrd::Relaxed);  //同步池总权重
        let r: usize = rand::thread_rng().gen(); // 由外部实现随机生成器， TODO
        let amount = async_w + sync_w;
        let w = if amount == 0 {
            0
        }else {
            r%amount
        };
        if w < sync_w {
            let mut lock = self.sync_pool.1.lock().unwrap();
            let w = lock.get_weight();
            if w != 0 {
                let r = Some(lock.pop_front(r%w));
                self.sync_pool.0.store(lock.get_weight(), AOrd::Relaxed);
                return r;
            }
        }
        let mut lock = self.async_pool.1.lock().unwrap();
        let w = lock.amount();
        if w != 0 {
            let r = Some(lock.remove_by_weight(r%w).0);
            self.async_pool.0.store(lock.amount(), AOrd::Relaxed);
            return r;
        }else {
            return None;
        }
    }

    /// pop a task , lock the queue of task if task is sync
    pub fn pop(&self) -> Option<Task<T>>{
        let async_w = self.async_pool.0.load(AOrd::Relaxed); //异步池总权重
        let sync_w = self.sync_pool.0.load(AOrd::Relaxed); //同步池总权重
        let r: usize = rand::thread_rng().gen();
        let amount = async_w + sync_w;
        let w =  if amount == 0 {
            0
        }else {
            r%amount
        };
        if w < sync_w {
            let mut lock = self.sync_pool.1.lock().unwrap();
            let w = lock.get_weight();
            if w != 0 {
                let (elem, index) = lock.pop_front_with_lock(r%w);
                self.sync_pool.0.store(lock.get_weight(), AOrd::Relaxed);
                return Some(Task::Sync(elem, index));
            }
        }
        let mut lock = self.async_pool.1.lock().unwrap();
        let w = lock.amount();
        if w != 0 {
            let r = Some(Task::Async(lock.remove_by_weight(r%w).0));
            self.async_pool.0.store(lock.amount(), AOrd::Relaxed);
            return r;
        }else {
            return None;
        }
    }

    pub fn remove_sync(&self, _index: &Arc<AtomicUsize>) {
        //TODO
        //self.sync_pool.1.lock().unwrap().remove();
    }

    pub fn remove_async(&self, index: &Arc<AtomicUsize>) -> Option<T> {
        self.async_pool.1.lock().unwrap().try_remove(index)
    }

    pub fn clear(&self) {
        self.sync_pool.1.lock().unwrap().clear();
        self.async_pool.1.lock().unwrap().clear();
        self.sync_pool.0.store(0, AOrd::Relaxed);
        self.async_pool.0.store(0, AOrd::Relaxed);
        self.delay_queue.clear();
    }

    pub fn len(&self) -> usize {
        let sync_pool = self.sync_pool.1.lock().unwrap();
        let async_pool = self.async_pool.1.lock().unwrap();
        sync_pool.len() + async_pool.len()
    }

    /// lock sync_queue
    pub fn lock_sync_queue(&self, id: &Arc<AtomicUsize>) -> Result<(), TaskPoolErr> {
        let mut lock = self.sync_pool.1.lock().unwrap();
        let r = lock.lock_queue(id);
        self.sync_pool.0.store(lock.get_weight(), AOrd::Relaxed);
        r
    }

    /// free lock sync_queue
    pub fn free_sync_queue(&self, id: &Arc<AtomicUsize>) -> Result<(), TaskPoolErr> {
        let mut lock = self.sync_pool.1.lock().unwrap();
        let r = lock.free_queue(id);
        self.sync_pool.0.store(lock.get_weight(), AOrd::Relaxed);
        r
    }
}

unsafe impl<T: Send> Send for TaskPool<T> {}
unsafe impl<T: Send> Sync for TaskPool<T> {}

impl<T: fmt::Debug> fmt::Debug for TaskPool<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let sync_pool = self.sync_pool.1.lock().unwrap();
        let async_pool = self.async_pool.1.lock().unwrap();
        write!(f, "TaskPool {{ sync_pool: ({}), async_pool: ({}) }}", sync_pool.len(), async_pool.len())
    }
}

// pub struct QueueLock<T: 'static>{
//     sync_pool: Arc<(AtomicUsize, Mutex<SyncPool<T>>)>,
//     index: Arc<AtomicUsize>,
//     weight: usize,
// }

// impl<T: 'static> Drop for QueueLock<T> {
//     fn drop(&mut self){
//         println!("drop--------------------------------{:?}", self.index);
//         let mut lock = self.sync_pool.1.lock().unwrap();
//         lock.free_lock(&self.index, self.weight);
//         self.sync_pool.0.store(lock.get_weight(), AOrd::Relaxed);
//     }
// }

//任务
pub enum Task<T: 'static> {
    Async(T),
    Sync(T, Arc<AtomicUsize>),
}

//任务类型
#[derive(Clone)]
pub enum TaskType {
    Async(usize),      //异步任务, Async(任务优先级, 能否删除)
    Sync(Arc<AtomicUsize>, Direction),       //同步任务Sync(队列id, push方向)
}

//同步任务push的方向
#[derive(Clone)]
pub enum Direction {
    Front,
    Back,
}
// map性能低， 考虑去掉map， weight_queues使用slab， TODO
pub struct SyncPool<T: 'static>{
    weight_queues: WeightTree<WeightQueue<T>>,
    len: usize,
}

unsafe impl<T: Send> Send for SyncPool<T> {}
unsafe impl<T: Send> Sync for SyncPool<T> {}

impl<T: 'static> SyncPool<T>{

    fn new() -> Self {
        SyncPool {
            weight_queues: WeightTree::new(),
            len: 0,
        }
    }

    //create queues, if id is exist, return false
    fn create_queue(&mut self, weight: usize, can_del: bool) -> Arc<AtomicUsize> {
        self.weight_queues.push(WeightQueue::new(weight, can_del), 0)
    }

    fn lock_queue(&mut self, id: &Arc<AtomicUsize>) -> Result<(), TaskPoolErr> {
        match self.weight_queues.get_mut(id) {
            Some(queues) => {
                if queues.is_lock() {
                    return Err(TaskPoolErr::SyncQueueLockExist(id.clone()));
                }
                queues.lock();
            },
            None => return Err(TaskPoolErr::NoSyncQueue(id.clone())),
        };
        self.weight_queues.update_weight(0, id);
        Ok(())
    }

    fn free_queue(&mut self, id: &Arc<AtomicUsize>) -> Result<(), TaskPoolErr> {
        let w = match self.weight_queues.get_mut(id) {
            Some(queues) => {
                if !queues.is_lock() {
                    return Err(TaskPoolErr::SyncQueueLockExist(id.clone()));
                }
                queues.free_lock();
                queues.get_weight()
            },
            None => return Err(TaskPoolErr::NoSyncQueue(id.clone())),
        };
        self.weight_queues.update_weight(w, id);
        Ok(())
    }

    //Find a queue with weight, Removes the first element from the queue and returns it, Painc if weight >= get_weight().
    fn pop_front(&mut self, weight: usize) -> T {
        let (r, weight, index) = {
            let queue = self.weight_queues.get_mut_by_weight(weight);
            (queue.0.pop_front().unwrap(), queue.0.get_weight(), queue.1.clone())  //如果能够根据权重取到队列， 必然能从队列中弹出元素
        };
        self.weight_queues.update_weight(weight, &index);
        self.len -= 1;
        r
    }

    //pop elements from specified queue, and not update weight, Painc if weight >= get_weight()
    fn pop_front_with_lock(&mut self, weight: usize) -> (T, Arc<AtomicUsize>) {
        let r = {
            let queue = self.weight_queues.get_mut_by_weight(weight);
            queue.0.lock();
            (queue.0.pop_front().unwrap(), queue.1.clone()) //如果能够根据权重取到队列， 必然能从队列中弹出元素
        };
        self.weight_queues.update_weight(0, &r.1);
        self.len -= 1;
        r
    }

    //Find a queue with weight, Removes the last element from the queue and returns it, or None if the queue is empty or the queue is not exist.
    fn _pop_back(&mut self, weight: usize) -> Option<T> {
        let (r, weight, index) = {
            let queue = match self.weight_queues.try_get_mut_by_weight(weight){
                Some(v) => {v},
                None => return None
            };
            (queue.0._pop_back(), queue.0.get_weight(), queue.1.clone())
        };
        self.weight_queues.update_weight(weight, &index);
        if r.is_some() {
            self.len -= 1;
        }
        r
    }

    //Append an element to the queue of the specified ID. return index, or None if the queue is FastQueue
    fn push_back(&mut self, id: &Arc<AtomicUsize>, task: T) -> Result<Option<Arc<AtomicUsize>>, TaskPoolErr> {
        let (index, weight) = match self.weight_queues.get_mut(&id) {
            Some(queue) => {
                self.len += 1;
                let index = queue.push_back(task);
                if queue.is_lock(){
                    return Ok(index);
                }
                (index, queue.get_weight())
            },
            None => return Err(TaskPoolErr::NoSyncQueue(id.clone())),
        };
        self.weight_queues.update_weight(weight, &id);
        Ok(index)
    }

    //Prepends an element to the queue of the specified ID. return index, or None if the queue is FastQueue
    fn push_front(&mut self, id: &Arc<AtomicUsize>, task: T) -> Result<Option<Arc<AtomicUsize>>, TaskPoolErr>{
        let (index, weight) = match self.weight_queues.get_mut(&id) {
            Some(queue) => {
                self.len += 1;
                let index = queue.push_front(task);
                if queue.is_lock(){
                    return Ok(index);
                }
                (index, queue.get_weight())
            },
            None => return Err(TaskPoolErr::NoSyncQueue(id.clone())),
        };
        self.weight_queues.update_weight(weight, &id);
        Ok(index)
    }

    //Prepends an element to the queue of the specified ID. return true, or false if the queue is VecQueue
    fn push_front_with_index(&mut self, id: &Arc<AtomicUsize>, task: T, index: &Arc<AtomicUsize>) -> Result<(), TaskPoolErr>{
        let weight = match self.weight_queues.get_mut(&id) {
            Some(queue) => {
                match queue.push_front_with_index(task, index){
                    true => {
                        self.len += 1;
                        if queue.is_lock(){
                            return Ok(());
                        }
                        queue.get_weight()
                    },
                    false => return Err(TaskPoolErr::NoSyncQueue(id.clone())),
                }
            },
            None => return Err(TaskPoolErr::NoSyncQueue(id.clone())),
        };
        self.weight_queues.update_weight(weight, &id);
        Ok(())
    }

    //Append an element to the queue of the specified ID. return true, or false if the queue is VecQueue
    fn push_back_with_index(&mut self, id: &Arc<AtomicUsize>, task: T, index: &Arc<AtomicUsize>) -> Result<(), TaskPoolErr>{
        let weight = match self.weight_queues.get_mut(&id) {
            Some(queue) => {
                match queue.push_back_with_index(task, index){
                    true => {
                        self.len += 1;
                        if queue.is_lock(){
                            return Ok(());
                        }
                        queue.get_weight()
                    },
                    false => return Err(TaskPoolErr::NoSyncQueue(id.clone())),
                }
            },
            None => return Err(TaskPoolErr::NoSyncQueue(id.clone())),
        };
        self.weight_queues.update_weight(weight, &id);
        Ok(())
    }

    //取队列的权重（所有任务的权重总值）
    fn get_weight(&self) -> usize{
        self.weight_queues.amount()
    }

    //移除指定id的队列
    fn _remove(&mut self, id: &Arc<AtomicUsize>) {
       self.len -= self.weight_queues.remove(&id).len();
    }

    fn try_remove(&mut self, id: &Arc<AtomicUsize>) -> bool {
       match self.weight_queues.try_remove(&id){
           Some(queue) => {
               self.len -= queue.len();
               true
           },
           None => false
       }
    }

    //清空同步任务池
    fn clear(&mut self) {
        self.weight_queues.clear();
        self.len = 0;
    }

    //长度
    pub fn len(&self) -> usize {
        self.len
    }
}

pub type AsyncPool<T> = WeightTree<T>;

pub enum DelayTask<T: 'static> {
    Async{
        priority: usize,
        async_pool: Arc<(AtomicUsize, Mutex<AsyncPool<T>>)>,
        task:  NonNull<T>,
    },//异步任务
    Sync{
        id: Arc<AtomicUsize>,
        direc: Direction,
        sync_pool: Arc<(AtomicUsize, Mutex<SyncPool<T>>)>,
        task:  NonNull<T>,
    }//同步任务Sync(队列id, push方向)
}

impl<T: 'static> Runer for DelayTask<T> {
    fn run(self, index: &Arc<AtomicUsize>){
        match self {
            DelayTask::Async { priority,async_pool,task } => {
                let mut lock = async_pool.1.lock().unwrap();
                lock.push_with_index(unsafe {task.as_ptr().read()} , priority, index);
                async_pool.0.store(lock.amount(), AOrd::Relaxed);
            },
            DelayTask::Sync { id, direc, sync_pool, task } => {
                match direc {
                    Direction::Front => {
                        let mut lock = sync_pool.1.lock().unwrap();
                        lock.push_front_with_index(&id, unsafe {task.as_ptr().read()}, &index).expect("delay task push fail,");
                        sync_pool.0.store(lock.get_weight(), AOrd::Relaxed);
                    },
                    Direction::Back => {
                        let mut lock = sync_pool.1.lock().unwrap();
                        lock.push_back_with_index(&id, unsafe {task.as_ptr().read()}, &index).expect("delay task push fail,");
                        sync_pool.0.store(lock.get_weight(), AOrd::Relaxed);
                    }
                }
            }
        }
    }
}

unsafe impl<T> Send for DelayTask<T> {}

enum Deque<T>{
    VecDeque(VecDeque<T>),
    FastDeque(FastDeque<T>)  //需要优化，后期改为slab， TODO
}

//权重队列， WeightQueue(权重, 队列)
struct WeightQueue<T>{
    weight_unit: usize, //单个任务权重
    weight: usize, //队列总权重
    queue: Deque<T>, //队列
    lock: bool,
}

impl<T> WeightQueue<T>{
    fn new(weight_unit: usize, can_del: bool) -> Self{
        let q = match can_del {
            true => Deque::FastDeque(FastDeque::new()),
            false => Deque::VecDeque(VecDeque::new())
        };
        WeightQueue{
            weight_unit: weight_unit,
            weight: 0,
            queue: q,
            lock: false
        }
    }

    fn _pop_back(&mut self) -> Option<T>{
        let r = match self.queue {
            Deque::FastDeque(ref mut queue) => queue.pop_back(),
            Deque::VecDeque(ref mut queue) => queue.pop_back(),
        };
        if r.is_some() {
            self.weight -= self.weight_unit;
        }
        r
    }

    fn pop_front(&mut self) -> Option<T>{
        let r = match self.queue {
            Deque::FastDeque(ref mut queue) => queue.pop_front(),
            Deque::VecDeque(ref mut queue) => queue.pop_front(),
        };
        if r.is_some() {
            self.weight -= self.weight_unit;
        }
        r
    }

    fn push_back(&mut self, task: T) -> Option<Arc<AtomicUsize>>{
        self.weight += self.weight_unit;
        match self.queue {
            Deque::FastDeque(ref mut queue) => return Some(Arc::new(AtomicUsize::new(queue.push_back(task)))),
            Deque::VecDeque(ref mut queue) => {queue.push_back(task); return None},
        }
    }

    fn push_front(&mut self, task: T) -> Option<Arc<AtomicUsize>>{
        self.weight += self.weight_unit;
        match self.queue {
            Deque::FastDeque(ref mut queue) => Some(Arc::new(AtomicUsize::new(queue.push_front(task)))),
            Deque::VecDeque(ref mut queue) => {queue.push_front(task); None},
        }
    }

    fn push_back_with_index(&mut self, task: T, index: &Arc<AtomicUsize>) -> bool{
        match self.queue {
            Deque::FastDeque(ref mut queue) => {self.weight += self.weight_unit; index.store(queue.push_back(task), AOrd::Relaxed); true},
            _ => false,
        }
    }

    fn push_front_with_index(&mut self, task: T, index: &Arc<AtomicUsize>) -> bool {
        match self.queue {
            Deque::FastDeque(ref mut queue) => {self.weight += self.weight_unit; index.store(queue.push_front(task), AOrd::Relaxed); true},
            _ => false,
        }
    }

    //取队列的权重（所有任务的权重总值）
    fn get_weight(&self) -> usize {
        self.weight
    }

    fn len(&self) -> usize{
        match self.queue {
            Deque::FastDeque(ref _queue) => 0,
            Deque::VecDeque(ref queue) => queue.len(),
        }
    }

    fn lock(&mut self) {
        self.lock = true;
    }

    fn free_lock(&mut self) {
        self.lock = false;
    }

    fn is_lock(&self) -> bool {
        self.lock
    }
}

#[cfg(test)]
use time::now_millis;
#[cfg(test)]
use std::thread;
#[cfg(test)]
use std::time::{Duration};

#[test]
fn test_sync(){
	let task_pool: Arc<TaskPool<u32>> = Arc::new(TaskPool::new(Timer::new(10)));
    let syncs:[u32; 5] = [100000, 100000, 100000, 100000, 100000];
    let mut id_arr = Vec::new();
    let async = 100000;

    let now = now_millis();
    for i in 0..syncs.len() {
        id_arr.push(task_pool.create_sync_queue(i + 1, false));
    }

    for i in 0..syncs.len() {
        for v in 0..syncs[i].clone() {
           task_pool.push_sync(v, &id_arr[i], Direction::Back).unwrap();
        }
    }
    println!("push sync back time{}",  now_millis() - now);

    let now = now_millis();
    for i in 0..async{
        task_pool.push_async(i, (i + 1) as usize);
    }
    println!("push async back time{}",  now_millis() - now);

    let mut max = async;
    //let mut max = 0;
    for i in 0..syncs.len() {
        max += syncs[i];
    }

    println!("task_pool len------len:{:?}, max:{}", task_pool, max);
    let now = now_millis();
    for _ in 0..max{
        match task_pool.pop().unwrap() {
            Task::Sync(_, index) => {
                task_pool.free_sync_queue(&index).unwrap();
            },
            _ => (),
        } ;
    }
    println!("task_pool len------len:{:?}", task_pool);
    println!("pop back time{}",  now_millis() - now);
}



#[test]
fn test_async(){
	let task_pool: Arc<TaskPool<u32>> = Arc::new(TaskPool::new(Timer::new(0)));
    let mut id_arr = Vec::new();

    for i in 0..5{
        id_arr.push(Arc::new(task_pool.create_sync_queue(i + 1, false)));
    }

    let now = now_millis();
    let count = Arc::new(AtomicUsize::new(0));
    for i in 0..5{
        let task_pool = task_pool.clone();
        let count = count.clone();
        let id = id_arr[i].clone();
        thread::spawn(move || {
            for v in 0..1000 {
                task_pool.push_sync(v, &id, Direction::Back).unwrap();
            }
            count.fetch_add(1, AOrd::Relaxed);
            if count.load(AOrd::Relaxed) == 10 {
                println!("push time{}",  now_millis() - now);
                pop(task_pool.clone());
            }
        });
    }
    for i in 0..5{
        let task_pool = task_pool.clone();
        let count = count.clone();
        thread::spawn(move || {
            for v in 0..1000 {
                let r = v * i;
                task_pool.push_async(r as u32, r + 1);
            }
            count.fetch_add(1, AOrd::Relaxed);
            if count.load(AOrd::Relaxed) == 10 {
                println!("push time{}",  now_millis() - now);
                pop(task_pool.clone());
            }
        });
    }

    thread::sleep(Duration::from_millis(1000));
}

#[cfg(test)]
fn pop (task_pool: Arc<TaskPool<u32>>){
    let now = now_millis();
    let count = Arc::new(AtomicUsize::new(0));
    println!("task_pool len------{:?}", task_pool);
    for _ in 0..10{
        let task_pool = task_pool.clone();
        let count = count.clone();
        thread::spawn(move || {
            for _ in 0..1000 {
                task_pool.pop();
            }
            count.fetch_add(1, AOrd::Relaxed);
            if count.load(AOrd::Relaxed) == 10 {
                println!("pop time{}",  now_millis() - now);
                println!("task_pool len------{:?}", task_pool);
            }
        });
    }
}


#[test]
fn lock_queue(){
    let task_pool: Arc<TaskPool<u32>> = Arc::new(TaskPool::new(Timer::new(0)));
    // task_pool.create_sync_queue(1, false);
    // task_pool.create_sync_queue(2, false);
    let id1 = task_pool.create_sync_queue(3, false);
    let id = task_pool.create_sync_queue(4, false);
    task_pool.lock_sync_queue(&id).unwrap();
    for i in 4..10 {
        task_pool.push_sync(i, &id, Direction::Back).unwrap();
    }

    for i in 4..10 {
        task_pool.push_sync(i, &id1, Direction::Back).unwrap();
    }

    for i in 4..10 {
        task_pool.push_sync(i, &id, Direction::Back).unwrap();
    }

    assert_eq!(task_pool.pop().is_some(), true);
    task_pool.free_sync_queue(&id).unwrap();
    task_pool.pop();
    task_pool.pop();
    task_pool.pop();
    task_pool.pop();
    task_pool.pop();
    task_pool.pop();
    task_pool.pop();
    task_pool.pop();
    // task_pool.free_sync_queue(&id);
    // assert_eq!(r.is_some(), true);
    // assert_eq!(task_pool.pop().is_some(), false);
}