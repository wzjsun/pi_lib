#![crate_type = "rlib"]
#![feature(rustc_private)]
#![feature(const_vec_new)]
#![feature(integer_atomics)]
#![feature(duration_extras)]
#![feature(custom_derive,asm,box_syntax,box_patterns)]
#![feature(pointer_methods)]
#![feature(core_intrinsics)]
#![feature(generators, generator_trait)]
#![feature(associated_type_defaults)]
#![feature(exclusive_range_pattern)]
#![feature(box_into_raw_non_null)]
#![feature(assoc_unix_epoch)]
#![feature(trait_alias)]
#![feature(const_fn)]
#![feature(nll)]
#![feature(fnbox)]
#[warn(unreachable_patterns)]

#[allow(dead_code,unused_variables,non_snake_case,unused_parens,unused_assignments,unused_unsafe,unused_imports)]

extern crate core;
extern crate fnv;
extern crate crc;
extern crate rand;

pub mod slab;
pub mod rc;
pub mod ordmap;
pub mod base58;
#[macro_use]
pub mod sbtree;
pub mod asbtree;
pub mod bon;
pub mod data_view;
pub mod atom;
pub mod sinfo;
pub mod util;
pub mod guid;
pub mod time;
pub mod cowlist;
pub mod heap;
pub mod wheel;
pub mod handler;
pub mod wtree;
pub mod adler32;
pub mod rsync;
pub mod gray;
pub mod task_pool;
pub mod timer;
pub mod fast_deque;
pub mod wcs;
//pub mod component;

#[macro_use]
extern crate lazy_static;