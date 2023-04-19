use crate::{
  kernel_wrapper::BPF_SKELETON, lqos_kernel::bpf, HostCounter,
  RttTrackingEntry,
};
use lqos_utils::XdpIpAddress;
use once_cell::sync::Lazy;
use std::{
  fs::File, io::Read, marker::PhantomData, os::fd::FromRawFd, fmt::Debug,
};
use thiserror::Error;
use zerocopy::FromBytes;

/// Represents a link to an eBPF defined iterator. The iterators
/// must be available in the BPF skeleton, and the skeleton must
/// be loaded. These are designed to be lazy-initialized on a
/// per-map basis. The `MAP_TRAFFIC` and `RTT_ITERATOR` types
/// implement this type.
/// 
/// Normal usage is to initialize the iterator and keep it around.
/// When you need to query the iterator, execute the `iter` method
/// and treat it as a normal Rust iterator.
struct BpfMapIterator<KEY, VALUE> {
  link: *mut bpf::bpf_link,
  _phantom: PhantomData<(KEY, VALUE)>,
}

// The BPF map is re-entrant and thread safe. There's no clean
// way to represent this in Rust, so we just mark it as such.
unsafe impl<KEY, VALUE> Sync for BpfMapIterator<KEY, VALUE> {}
unsafe impl<KEY, VALUE> Send for BpfMapIterator<KEY, VALUE> {}

impl<KEY, VALUE> BpfMapIterator<KEY, VALUE> {
  /// Create a new link to an eBPF map, that *must* have an iterator
  /// function defined in the eBPF program - and exposed in the
  /// skeleton.
  /// 
  /// # Safety
  /// 
  /// * This is unsafe, it relies on the skeleton having been properly
  ///   initialized prior to using this type.
  /// 
  /// # Arguments
  /// 
  /// * `program` - The eBPF program that points to the iterator function.
  /// * `map` - The eBPF map that the iterator function will iterate over.
  fn new(
    program: *mut bpf::bpf_program,
    map: *mut bpf::bpf_map,
  ) -> Result<Self, BpfIteratorError> {
    let link = unsafe { bpf::setup_iterator_link(program, map) };
    if !link.is_null() {
      Ok(Self { link, _phantom: PhantomData })
    } else {
      Err(BpfIteratorError::FailedToLink)
    }
  }

  /// Create a "link file descriptor", connecting the eBPF iterator
  /// to a Linux file descriptor. This instantiates the iterator
  /// in the kernel and allows us to read from it.
  fn as_file(&self) -> Result<File, BpfIteratorError> {
    let link_fd = unsafe { bpf::bpf_link__fd(self.link) };
    let iter_fd = unsafe { bpf::bpf_iter_create(link_fd) };
    if iter_fd < 0 {
      log::error!("Unable to create map file descriptor");
      Err(BpfIteratorError::FailedToCreateFd)
    } else {
      unsafe { Ok(File::from_raw_fd(iter_fd)) }
    }
  }

  /// Transform the iterator into a Rust iterator. This can then
  /// be used like a regular iterator. The iterator owns the
  /// file's buffer and provides references. The iterator MUST
  /// outlive the functions that use it (you can clone all you
  /// like).
  fn iter(&self) -> Result<BpfMapIter<KEY, VALUE>, BpfIteratorError> {
    let mut file = self.as_file()?;
    let mut buf = Vec::new();
    let bytes_read = file.read_to_end(&mut buf);
    match bytes_read {
      Ok(_) => Ok(BpfMapIter::new(buf)),
      Err(e) => {
        log::error!("Unable to read from kernel map iterator file");
        log::error!("{e:?}");
        Err(BpfIteratorError::UnableToCreateIterator)
      }
    }
  }
}

/// When the iterator is dropped, we need to destroy the link.
/// This is handled by the kernel when the program is unloaded.
impl<KEY, VALUE> Drop for BpfMapIterator<KEY, VALUE> {
  fn drop(&mut self) {
    unsafe {
      bpf::bpf_link__destroy(self.link);
    }
  }
}

/// Rust iterator for reading data from eBPF map iterators.
/// Transforms the data into the appropriate types, and returns
/// a tuple of the key, and a vector of values (1 per CPU for 
/// CPU_MAP types, 1 entry for all others).
pub(crate) struct BpfMapIter<K, V> {
  buffer: Vec<u8>,
  index: usize,
  _phantom: PhantomData<(K, V)>,
  num_cpus: u32,
}

impl<K, V> BpfMapIter<K, V> {
  const KEY_SIZE: usize = std::mem::size_of::<K>();
  const VALUE_SIZE: usize = std::mem::size_of::<V>();
  const TOTAL_SIZE: usize = Self::KEY_SIZE + Self::VALUE_SIZE;

  /// Transforms the buffer into a Rust iterator. The buffer
  /// is *moved* into the iterator, which retains ownership
  /// throughout.
  fn new(buffer: Vec<u8>) -> Self {
    let first_four : [u8; 4] = [buffer[0], buffer[1], buffer[2], buffer[3]];
    let num_cpus = u32::from_ne_bytes(first_four);
    //println!("CPUs: {num_cpus}");

    Self {
      buffer,
      index: std::mem::size_of::<i32>(),
      _phantom: PhantomData,
      num_cpus,
    }
  }
}

impl<K, V> Iterator for BpfMapIter<K, V>
where
  K: FromBytes + Debug,
  V: FromBytes + Debug,
{
  type Item = (K, Vec<V>);

  fn next(&mut self) -> Option<Self::Item> {
    if self.index + Self::TOTAL_SIZE <= self.buffer.len() {      
      let key = K::read_from(&self.buffer[self.index..self.index + Self::KEY_SIZE]);
      self.index += Self::KEY_SIZE;
      let mut vals = Vec::new();
      for _ in 0..self.num_cpus {
        let value = V::read_from(
          &self.buffer
            [self.index ..self.index + Self::VALUE_SIZE],
        );
        vals.push(value.unwrap());
        self.index += Self::VALUE_SIZE;
      }
      //println!("{key:?} {vals:?}");
      Some((key.unwrap(), vals))
    } else {
      None
    }
  }
}

#[derive(Debug, Error)]
enum BpfIteratorError {
  #[error("Failed to create iterator link")]
  FailedToLink,
  #[error("Failed to create file descriptor")]
  FailedToCreateFd,
  #[error("Iterator error")]
  UnableToCreateIterator,
}

static mut MAP_TRAFFIC: Lazy<
  Option<BpfMapIterator<XdpIpAddress, HostCounter>>,
> = Lazy::new(|| None);

static mut RTT_TRACKER: Lazy<
  Option<BpfMapIterator<XdpIpAddress, RttTrackingEntry>>,
> = Lazy::new(|| None);

pub unsafe fn iterate_throughput(callback: &mut dyn FnMut(&XdpIpAddress, &[HostCounter])) {
  if MAP_TRAFFIC.is_none() {
    let lock = BPF_SKELETON.lock().unwrap();
    if let Some(skeleton) = lock.as_ref() {
      let skeleton = skeleton.get_ptr();
      if let Ok(iter) = unsafe {
        BpfMapIterator::new(
          (*skeleton).progs.throughput_reader,
          (*skeleton).maps.map_traffic,
        )
      } {
        *MAP_TRAFFIC = Some(iter);
      }
    }
  }

  if let Some(iter) = MAP_TRAFFIC.as_mut() {
    iter.iter().unwrap().for_each(|(k, v)| {
      //println!("{:?} {:?}", k, v);
      callback(&k, &v);
    });
  }
}

pub unsafe fn iterate_rtt(callback: &mut dyn FnMut(&XdpIpAddress, &RttTrackingEntry)) {
  if RTT_TRACKER.is_none() {
    let lock = BPF_SKELETON.lock().unwrap();
    if let Some(skeleton) = lock.as_ref() {
      let skeleton = skeleton.get_ptr();
      if let Ok(iter) = unsafe {
        BpfMapIterator::new(
          (*skeleton).progs.rtt_reader,
          (*skeleton).maps.rtt_tracker,
        )
      } {
        *RTT_TRACKER = Some(iter);
      }
    }
  }

  if let Some(iter) = RTT_TRACKER.as_mut() {
    iter.iter().unwrap().for_each(|(k, v)| {
      callback(&k, &v[0]); // Not per-CPU
    });
  }
}
