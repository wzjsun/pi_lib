/**
 * 时间函数库
 * 提供全局时间算法
 * 时间同步算法！启动时间也要记录。
第1次的时间校准：100s +500ms
第2次的时间校准：200s +400ms，这时需要后退100ms，预计在50s内消化这个100ms，算法为：
T = Now - LastTime;
if
	T >= FixFixTime ->
		T + Fix;
	true ->
		T + Fix - ((FixFixTime - T) * FixFix div FixFixTime)
end
Now 200s
 */

use std::time::Instant;
use std::time::SystemTime;


lazy_static! {
	static ref START: Instant = Instant::now();
	static ref START_SECS: u64 = SystemTime::UNIX_EPOCH.elapsed().unwrap().as_secs();
}

// 启动后运行的纳秒数
pub fn now_nanos() -> u64 {
	let d = START.elapsed();
	d.as_secs() * 1000_000_000 + d.subsec_nanos() as u64
}
// 启动后运行的微秒数
pub fn now_micros() -> u64 {
	let d = START.elapsed();
	d.as_secs() * 1000_000 + d.subsec_micros() as u64
}
// 启动后运行的毫秒数
pub fn now_millis() -> u64 {
	let d = START.elapsed();
	d.as_secs() * 1000 + d.subsec_millis() as u64
}
// 启动后运行的秒数
pub fn now_second() -> u64 {
	START.elapsed().as_secs()
}
// 当前进程的启动时间，单位：秒
pub fn start_secs() -> u64 {
	*START_SECS
}
