// vim: foldmarker=<([{,}])> foldmethod=marker

// Module level Doc <([{
//! The module includes utilities for other modules.

use std::ptr::{self, with_exposed_provenance_mut, write_unaligned};
// }])>

// save framework <([{
pub struct SaveUtils();

impl SaveUtils {
    pub fn prefix_len(mut data: Vec<u8>) -> Vec<u8> {
        let len = data.len();
        let mut ret: Vec<u8> = Vec::new();
        ret.reserve(size_of::<usize>() + len);
        // Safety: to make return sequence more compact.
        unsafe {
            let p = ret.as_mut_ptr() as *mut usize;
            write_unaligned(p, len);
            ret.set_len(size_of::<usize>());
        }
        ret.append(&mut data);
        ret
    }

    pub fn typename_data(ty_name: Vec<u8>, data: Vec<u8>) -> Vec<u8> {
        let mut ret = SaveUtils::prefix_len(ty_name);
        ret.append(&mut SaveUtils::prefix_len(data));
        ret
    }

    pub fn get_typename_data(buf: &[u8], mut pos: usize) -> ((usize, usize), (usize, usize)) {
        let v = &buf[pos..];
        let (name_start, name_end) = SaveUtils::recalculate(v, pos);
        pos = name_end;
        let v = &buf[pos..];
        let (data_start, data_end) = SaveUtils::recalculate(v, pos);
        ((name_start, name_end), (data_start, data_end))
    }

    pub fn read_prefix_len(data: &[u8]) -> usize {
        // Safety: read len which is stored compactly.
        unsafe {
            let p = data.as_ptr() as *const usize;
            ptr::read_unaligned(p)
        }
    }

    pub fn recalculate(data: &[u8], mut end: usize) -> (usize, usize) {
        let len = SaveUtils::read_prefix_len(data);
        let start = end + size_of::<usize>();
        end = start + len;
        (start, end)
    }
}
// }])>

pub fn box2usize<T>(val: T) -> usize {
    Box::into_raw(Box::new(val)).expose_provenance()
}
pub unsafe fn usize2box<T>(ptr: usize) -> Box<T> {
    unsafe { Box::<T>::from_raw(with_exposed_provenance_mut(ptr)) }
}
