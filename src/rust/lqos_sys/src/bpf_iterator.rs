use crate::{
    HostCounter,
    bpf_map::BpfMap,
    flowbee_data::{FlowbeeData, FlowbeeKey},
    kernel_wrapper::BPF_SKELETON,
    lqos_kernel::bpf,
};
use lqos_utils::XdpIpAddress;
use parking_lot::Mutex;
use std::sync::OnceLock;
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
                    return Ok(());
                }
                if bytes < 8 {
                    error!("Kernel iterator buffer too small ({bytes} bytes)");
                    return Err(BpfIteratorError::UnableToCreateIterator);
                }

                let Ok(first_four_bytes): Result<[u8; 4], _> = buf[0..4].try_into() else {
                    return Err(BpfIteratorError::UnableToCreateIterator);
                };
                let num_cpus = u32::from_ne_bytes(first_four_bytes) as usize;
                if num_cpus == 0 || num_cpus > 4096 {
                    error!("Invalid NUM_CPUS value from iterator: {num_cpus}");
                    return Err(BpfIteratorError::UnableToCreateIterator);
                }

                let mut index = 8;
                while index + Self::KEY_SIZE <= buf.len() {
                    let key_start = index;
                    let key_end = key_start + Self::KEY_SIZE;
                    let key_slice = &buf[key_start..key_end];
                    index = key_end;

                    let values_len = num_cpus * Self::VALUE_SIZE;
                    if index + values_len > buf.len() {
                        error!(
                            "Truncated iterator buffer (need {} bytes, have {})",
                            index + values_len,
                            buf.len()
                        );
                        break;
                    }
                    let value_slice = &buf[index..index + values_len];
                    index += values_len;

                    let Ok(key) = KEY::read_from_bytes(key_slice) else {
                        error!("Failed to decode iterator key");
                        continue;
                    };

                    let mut values: Vec<VALUE> = Vec::with_capacity(num_cpus);
                    for cpu in 0..num_cpus {
                        let start = cpu * Self::VALUE_SIZE;
                        let end = start + Self::VALUE_SIZE;
                        let chunk = &value_slice[start..end];
                        let Ok(value) = VALUE::read_from_bytes(chunk) else {
                            error!("Failed to decode iterator value (cpu={cpu})");
                            continue;
                        };
                        values.push(value);
                    }

                    callback(&key, &values);
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

                    let value_start = key_end;
                    let value_end = value_start + Self::VALUE_SIZE;
                    let value_slice = &buf[value_start..value_end];
                    let Ok(key) = KEY::read_from_bytes(key_slice) else {
                        error!("Failed to decode iterator key");
                        index += Self::TOTAL_SIZE;
                        continue;
                    };
                    let Ok(value) = VALUE::read_from_bytes(value_slice) else {
                        error!("Failed to decode iterator value");
                        index += Self::TOTAL_SIZE;
                        continue;
                    };

                    callback(&key, &value);

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

static MAP_TRAFFIC: OnceLock<
    Mutex<Result<BpfMapIterator<XdpIpAddress, HostCounter>, BpfIteratorError>>,
> = OnceLock::new();

static FLOWBEE_TRACKER: OnceLock<
    Mutex<Result<BpfMapIterator<FlowbeeKey, FlowbeeData>, BpfIteratorError>>,
> = OnceLock::new();

pub unsafe fn iterate_throughput(callback: &mut dyn FnMut(&XdpIpAddress, &[HostCounter])) {
    let traffic_map = MAP_TRAFFIC.get_or_init(|| {
        let lock = BPF_SKELETON.lock();
        let Some(skeleton) = lock.as_ref() else { return Mutex::new(Err(BpfIteratorError::FailedToLink)) };
        let skeleton = skeleton.get_ptr();
        let iter = unsafe {
            BpfMapIterator::new((*skeleton).progs.throughput_reader, (*skeleton).maps.map_traffic)
        };
        Mutex::new(iter)
    });

    {
        let iter = traffic_map.lock();
        match &*iter {
            Ok(iter) => {
                if let Err(e) = iter.for_each_per_cpu(callback) {
                    error!("Throughput iterator error: {e:?}");
                }
            }
            Err(e) => error!("Throughput iterator unavailable: {e:?}"),
        }
    }
}

/// Iterate through the Flows 2 system tracker, retrieving all flows
pub fn iterate_flows(callback: &mut dyn FnMut(&FlowbeeKey, &FlowbeeData)) {
    let flowbee_tracker = FLOWBEE_TRACKER.get_or_init(|| {
        let lock = BPF_SKELETON.lock();
        let Some(skeleton) = lock.as_ref() else { return Mutex::new(Err(BpfIteratorError::FailedToLink)) };
        let skeleton = skeleton.get_ptr();
        let iter = unsafe { BpfMapIterator::new((*skeleton).progs.flow_reader, (*skeleton).maps.flowbee) };
        Mutex::new(iter)
    });

    {
        let iter = flowbee_tracker.lock();
        match &*iter {
            Ok(iter) => {
                if let Err(e) = iter.for_each(callback) {
                    error!("Flowbee iterator error: {e:?}");
                }
            }
            Err(e) => error!("Flowbee iterator unavailable: {e:?}"),
        }
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
