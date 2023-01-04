#![allow(dead_code)]
use anyhow::{Error, Result};
use libbpf_sys::{
    bpf_map_delete_elem, bpf_map_get_next_key, bpf_map_lookup_elem, bpf_map_update_elem,
    bpf_obj_get, BPF_NOEXIST,
};
use std::{
    ffi::{c_void, CString},
    marker::PhantomData,
    ptr::null_mut,
};

/// Represents an underlying BPF map, accessed via the filesystem.
/// `BpfMap` *only* talks to shared (not PER-CPU) variants of maps.
/// 
/// `K` is the *key* type, indexing the map.
/// `V` is the *value* type, and must exactly match the underlying C data type.
pub(crate) struct BpfMap<K, V> {
    fd: i32,
    _key_phantom: PhantomData<K>,
    _val_phantom: PhantomData<V>,
}

impl<K, V> BpfMap<K, V>
where
    K: Default + Clone,
    V: Default + Clone,
{
    /// Connect to a BPF map via a filename. Connects the internal
    /// file descriptor, which is held until the structure is
    /// dropped.
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

    /// Iterates the undlering BPF map, and adds the results
    /// to a vector. Each entry contains a `key, value` tuple.
    pub(crate) fn dump_vec(&self) -> Vec<(K, V)> {
        let mut result = Vec::new();

        let mut prev_key: *mut K = null_mut();
        let mut key: K = K::default();
        let key_ptr: *mut K = &mut key;
        let mut value = V::default();
        let value_ptr: *mut V = &mut value;

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

    /// Inserts an entry into a BPF map.
    /// Use this sparingly, because it briefly pauses XDP access to the
    /// underlying map (through internal locking we can't reach from
    /// userland).
    /// 
    /// ## Arguments
    /// 
    /// * `key` - the key to insert.
    /// * `value` - the value to insert.
    /// 
    /// Returns Ok if insertion succeeded, a generic error (no details yet)
    /// if it fails.
    pub(crate) fn insert(&mut self, key: &mut K, value: &mut V) -> Result<()> {
        let key_ptr: *mut K = key;
        let val_ptr: *mut V = value;
        let err = unsafe {
            bpf_map_update_elem(
                self.fd,
                key_ptr as *mut c_void,
                val_ptr as *mut c_void,
                BPF_NOEXIST.into(),
            )
        };
        if err != 0 {
            Err(Error::msg(format!("Unable to insert into map ({err})")))
        } else {
            Ok(())
        }
    }

    /// Deletes an entry from the underlying eBPF map.
    /// Use this sparingly, it locks the underlying map in the
    /// kernel. This can cause *long* delays under heavy load.
    /// 
    /// ## Arguments
    /// 
    /// * `key` - the key to delete.
    /// 
    /// Return `Ok` if deletion succeeded.
    pub(crate) fn delete(&mut self, key: &mut K) -> Result<()> {
        let key_ptr: *mut K = key;
        let err = unsafe { bpf_map_delete_elem(self.fd, key_ptr as *mut c_void) };
        if err != 0 {
            Err(Error::msg("Unable to delete from map"))
        } else {
            Ok(())
        }
    }

    /// Delete all entries in the underlying eBPF map.
    /// Use this sparingly, it locks the underlying map. Under
    /// heavy load, it WILL eventually terminate - but it might
    /// take a very long time. Only use this for cleaning up
    /// sparsely allocated map data.
    pub(crate) fn clear(&mut self) -> Result<()> {
        loop {
            let mut key = K::default();
            let mut prev_key: *mut K = null_mut();
            unsafe {
                let key_ptr: *mut K = &mut key;
                while bpf_map_get_next_key(self.fd, prev_key as *mut c_void, key_ptr as *mut c_void)== 0
                {
                    bpf_map_delete_elem(self.fd, key_ptr as *mut c_void);
                    prev_key = key_ptr;
                }
            }

            key = K::default();
            prev_key = null_mut();
            unsafe {
                let key_ptr: *mut K = &mut key;
                if bpf_map_get_next_key(self.fd, prev_key as *mut c_void, key_ptr as *mut c_void) != 0 {
                    break;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn clear_no_repeat(&mut self) -> Result<()> {
        let mut key = K::default();
        let mut prev_key: *mut K = null_mut();
        unsafe {
            let key_ptr: *mut K = &mut key;
            while bpf_map_get_next_key(self.fd, prev_key as *mut c_void, key_ptr as *mut c_void)== 0
            {
                bpf_map_delete_elem(self.fd, key_ptr as *mut c_void);
                prev_key = key_ptr;
            }
        }
        Ok(())
    }
}

impl<K, V> Drop for BpfMap<K, V> {
    fn drop(&mut self) {
        let _ = nix::unistd::close(self.fd);
    }
}
