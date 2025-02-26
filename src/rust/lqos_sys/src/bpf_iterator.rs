use crate::{
    HostCounter,
    bpf_map::BpfMap,
    flowbee_data::{FlowbeeData, FlowbeeKey},
    kernel_wrapper::BPF_SKELETON,
    lqos_kernel::bpf,
};
use lqos_utils::XdpIpAddress;
use std::sync::{Mutex, OnceLock};
use std::{fmt::Debug, fs::File, io::Read, marker::PhantomData, os::fd::FromRawFd};
use thiserror::Error;
use tracing::error;
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

impl<KEY, VALUE> BpfMapIterator<KEY, VALUE>
where
    KEY: FromBytes + Debug + Clone,
    VALUE: FromBytes + Debug + Clone + Default,
{
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
            Ok(Self {
                link,
                _phantom: PhantomData,
            })
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
            error!("Unable to create map file descriptor");
            Err(BpfIteratorError::FailedToCreateFd)
        } else {
            unsafe { Ok(File::from_raw_fd(iter_fd)) }
        }
    }

    const KEY_SIZE: usize = std::mem::size_of::<KEY>();
    const VALUE_SIZE: usize = std::mem::size_of::<VALUE>();
    const TOTAL_SIZE: usize = Self::KEY_SIZE + Self::VALUE_SIZE;

    fn for_each_per_cpu(
        &self,
        callback: &mut dyn FnMut(&KEY, &[VALUE]),
    ) -> Result<(), BpfIteratorError> {
        let mut file = self.as_file()?;
        let mut buf = Vec::new();
        let bytes_read = file.read_to_end(&mut buf);
        match bytes_read {
            Err(e) => {
                error!("Unable to read from kernel map iterator file");
                error!("{e:?}");
                Err(BpfIteratorError::UnableToCreateIterator)
            }
            Ok(bytes) => {
                if bytes == 0 {
                    // Not having any data is not an error
                    return Ok(());
                }
                let first_four_bytes: [u8; 4] = [buf[0], buf[1], buf[2], buf[3]];
                let num_cpus = u32::from_ne_bytes(first_four_bytes) as usize;
                let mut index = 8;
                while index + Self::TOTAL_SIZE <= buf.len() {
                    let key_start = index;
                    let key_end = key_start + Self::KEY_SIZE;
                    let key_slice = &buf[key_start..key_end];
                    //println!("{:?}", unsafe { &key_slice.align_to::<KEY>() });
                    let (_head, key, _tail) = unsafe { &key_slice.align_to::<KEY>() };

                    let value_start = key_end;
                    let value_end = value_start + (num_cpus * Self::VALUE_SIZE);
                    let value_slice = &buf[value_start..value_end];
                    //println!("{:?}", unsafe { &value_slice.align_to::<VALUE>() });
                    let (_head, values, _tail) = unsafe { &value_slice.align_to::<VALUE>() };
                    debug_assert_eq!(values.len(), num_cpus);

                    callback(&key[0], values);

                    index += Self::KEY_SIZE + (num_cpus * Self::VALUE_SIZE);
                }
                Ok(())
            }
        }
    }

    fn for_each(&self, callback: &mut dyn FnMut(&KEY, &VALUE)) -> Result<(), BpfIteratorError> {
        let mut file = self.as_file()?;
        let mut buf = Vec::new();
        let bytes_read = file.read_to_end(&mut buf);
        match bytes_read {
            Err(e) => {
                error!("Unable to read from kernel map iterator file");
                error!("{e:?}");
                Err(BpfIteratorError::UnableToCreateIterator)
            }
            Ok(_) => {
                let mut index = 0;
                while index + Self::TOTAL_SIZE <= buf.len() {
                    let key_start = index;
                    let key_end = key_start + Self::KEY_SIZE;
                    let key_slice = &buf[key_start..key_end];
                    let (_head, key, _tail) = unsafe { &key_slice.align_to::<KEY>() };

                    let value_start = key_end;
                    let value_end = value_start + Self::VALUE_SIZE;
                    let value_slice = &buf[value_start..value_end];
                    let (_head, values, _tail) = unsafe { &value_slice.align_to::<VALUE>() };

                    if !key.is_empty() && !values.is_empty() {
                        callback(&key[0], &values[0]);
                    } else {
                        error!("Empty key or value found in iterator");
                        if key.is_empty() {
                            error!("Empty key");
                        }
                        if values.is_empty() {
                            error!("Empty value");
                        }
                    }

                    index += Self::KEY_SIZE + Self::VALUE_SIZE;
                }
                Ok(())
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

#[derive(Debug, Error)]
enum BpfIteratorError {
    #[error("Failed to create iterator link")]
    FailedToLink,
    #[error("Failed to create file descriptor")]
    FailedToCreateFd,
    #[error("Iterator error")]
    UnableToCreateIterator,
}

static MAP_TRAFFIC: OnceLock<Mutex<BpfMapIterator<XdpIpAddress, HostCounter>>> = OnceLock::new();

static FLOWBEE_TRACKER: OnceLock<Mutex<BpfMapIterator<FlowbeeKey, FlowbeeData>>> = OnceLock::new();

pub unsafe fn iterate_throughput(callback: &mut dyn FnMut(&XdpIpAddress, &[HostCounter])) {
    let traffic_map = MAP_TRAFFIC.get_or_init(|| {
        let lock = BPF_SKELETON.lock().unwrap();
        let Some(skeleton) = lock.as_ref() else {
            panic!("Failed to create throughput iterator");
        };
        let skeleton = skeleton.get_ptr();
        let Ok(iter) = (unsafe {
            BpfMapIterator::new(
                (*skeleton).progs.throughput_reader,
                (*skeleton).maps.map_traffic,
            )
        }) else {
            panic!("Failed to create throughput iterator");
        };
        Mutex::new(iter)
    });

    if let Ok(iter) = traffic_map.lock() {
        let _ = iter.for_each_per_cpu(callback);
    }
}

/// Iterate through the Flows 2 system tracker, retrieving all flows
pub fn iterate_flows(callback: &mut dyn FnMut(&FlowbeeKey, &FlowbeeData)) {
    let flowbee_tracker = FLOWBEE_TRACKER.get_or_init(|| {
        let lock = BPF_SKELETON.lock().unwrap();
        let Some(skeleton) = lock.as_ref() else {
            panic!("Failed to create flowbee iterator");
        };
        let skeleton = skeleton.get_ptr();
        let Ok(iter) = (unsafe {
            BpfMapIterator::new((*skeleton).progs.flow_reader, (*skeleton).maps.flowbee)
        }) else {
            panic!("Failed to create flowbee iterator");
        };
        Mutex::new(iter)
    });

    if let Ok(iter) = flowbee_tracker.lock() {
        let _ = iter.for_each(callback);
    }
}

/// Adjust flows to have status 2 - already processed
///
// Arguments: the list of flow keys to expire
pub fn end_flows(flows: &mut [FlowbeeKey]) -> anyhow::Result<()> {
    let mut map = BpfMap::<FlowbeeKey, FlowbeeData>::from_path("/sys/fs/bpf/flowbee")?;
    let mut keys = flows.iter().map(|k| k.clone()).collect();

    map.clear_bulk_keys(&mut keys)?;

    Ok(())
}

/// Expire all throughput data for the given keys
/// This uses the bulk delete method, which is faster than
/// the per-row method due to only having one lock.
pub fn expire_throughput(mut keys: Vec<XdpIpAddress>) -> anyhow::Result<()> {
    let mut map = BpfMap::<XdpIpAddress, HostCounter>::from_path("/sys/fs/bpf/map_traffic")?;
    map.clear_bulk_keys(&mut keys)?;
    Ok(())
}
