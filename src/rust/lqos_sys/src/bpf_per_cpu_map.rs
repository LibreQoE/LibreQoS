use anyhow::{Error, Result};
use libbpf_sys::{
    bpf_map_get_next_key, bpf_map_lookup_elem, bpf_obj_get, libbpf_num_possible_cpus,
};
use std::fmt::Debug;
use std::{
    ffi::{c_void, CString},
    marker::PhantomData,
    ptr::null_mut,
};

/// Represents an underlying BPF map, accessed via the filesystem.
/// `BpfMap` *only* talks to PER-CPU variants of maps.
/// 
/// `K` is the *key* type, indexing the map.
/// `V` is the *value* type, and must exactly match the underlying C data type.
pub(crate) struct BpfPerCpuMap<K, V> {
    fd: i32,
    _key_phantom: PhantomData<K>,
    _val_phantom: PhantomData<V>,
}

impl<K, V> BpfPerCpuMap<K, V>
where
    K: Default + Clone,
    V: Default + Clone + Debug,
{
    /// Connect to a PER-CPU BPF map via a filename. Connects the internal
    /// file descriptor, which is held until the structure is
    /// dropped. The index of the CPU is *not* specified.
    pub(crate) fn from_path(filename: &str) -> Result<Self> {
        let filename_c = CString::new(filename)?;
        let fd = unsafe { bpf_obj_get(filename_c.as_ptr()) };
        if fd < 0 {
            Err(Error::msg("Unable to open BPF map"))
        } else {
            Ok(Self {
                fd,
                _key_phantom: PhantomData,
                _val_phantom: PhantomData,
            })
        }
    }

    /// Iterates the entire contents of the underlying eBPF per-cpu map.
    /// Each iteration returns one entry per CPU, even if there isn't a
    /// CPU-local map entry. Each result is therefore returned as one
    /// key and a vector of values.
    pub(crate) fn dump_vec(&self) -> Vec<(K, Vec<V>)> {
        let mut result = Vec::new();
        let num_cpus = unsafe { libbpf_num_possible_cpus() };

        let mut prev_key: *mut K = null_mut();
        let mut key: K = K::default();
        let key_ptr: *mut K = &mut key;
        let mut value = vec![V::default(); num_cpus as usize];
        let value_ptr = value.as_mut_ptr();

        unsafe {
            while bpf_map_get_next_key(self.fd, prev_key as *mut c_void, key_ptr as *mut c_void)
                == 0
            {
                bpf_map_lookup_elem(self.fd, key_ptr as *mut c_void, value_ptr as *mut c_void);
                result.push((key.clone(), value.clone()));
                prev_key = key_ptr;
            }
        }

        result
    }
}

impl<K, V> Drop for BpfPerCpuMap<K, V> {
    fn drop(&mut self) {
        let _ = nix::unistd::close(self.fd);
    }
}
