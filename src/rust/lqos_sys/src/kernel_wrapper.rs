use crate::lqos_kernel::{
    AttachedPrograms, InterfaceDirection, attach_xdp_and_tc_to_interface,
    bpf::{self, ring_buffer_sample_fn},
    unload_xdp_from_interface,
};
use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// Safer wrapper around pointers to `bpf::lqos_kern`. It really isn't
/// a great idea to be passing mutable pointers around like this, but the C
/// world insists on it.
pub(crate) struct LqosKernBpfWrapper {
    ptr: *mut bpf::lqos_kern,
    kprobe_link: *mut bpf::bpf_link,
}

impl LqosKernBpfWrapper {
    pub(crate) fn get_ptr(&self) -> *mut bpf::lqos_kern {
        self.ptr
    }
}

unsafe impl Sync for LqosKernBpfWrapper {}
unsafe impl Send for LqosKernBpfWrapper {}

impl Drop for LqosKernBpfWrapper {
    fn drop(&mut self) {
        unsafe {
            bpf::destroy_bpf_link(self.kprobe_link);
        }
    }
}

pub(crate) static BPF_SKELETON: Lazy<Mutex<Option<LqosKernBpfWrapper>>> =
    Lazy::new(|| Mutex::new(None));

/// A wrapper-type that stores the interfaces to which the XDP and TC programs should
/// be attached. Performs the attachment process, and hooks "drop" to unattach the
/// programs when the structure falls out of scope.
pub struct LibreQoSKernels {
    to_internet: String,
    to_isp: String,
    on_a_stick: bool,
}

impl LibreQoSKernels {
    /// Create a new `LibreQosKernels` structure, using the specified interfaces.
    /// Returns Ok(self) if attaching to the XDP/TC interfaces succeeded, otherwise
    /// returns an error containing a string describing what went wrong.
    ///
    /// Outputs progress to `stdio` during execution, and detailed errors to `stderr`.
    ///
    /// ## Arguments
    ///
    /// * `to_internet` - the name of the Internet-facing interface (e.g. `eth1`).
    /// * `to_isp` - the name of the ISP-network facing interface (e.g. `eth2`).
    /// * `heimdall_event_handler` - C function pointer to the ringbuffer
    ///   event handler exported by Heimdall.
    pub fn new<S: ToString>(
        to_internet: S,
        to_isp: S,
        heimdall_event_handler: ring_buffer_sample_fn,
        flowbee_event_handler: ring_buffer_sample_fn,
    ) -> anyhow::Result<Self> {
        let kernel = Self {
            to_internet: to_internet.to_string(),
            to_isp: to_isp.to_string(),
            on_a_stick: false,
        };
        let to_internet_ifindex =
            crate::lqos_kernel::interface_name_to_index(&kernel.to_internet)? as i32;
        let to_isp_ifindex = crate::lqos_kernel::interface_name_to_index(&kernel.to_isp)? as i32;
        let AttachedPrograms {
            skeleton,
            kprobe_link,
        } = attach_xdp_and_tc_to_interface(
            &kernel.to_internet,
            to_internet_ifindex,
            to_isp_ifindex,
            true,
            InterfaceDirection::Internet,
            heimdall_event_handler,
            flowbee_event_handler,
        )?;
        attach_xdp_and_tc_to_interface(
            &kernel.to_isp,
            to_internet_ifindex,
            to_isp_ifindex,
            false,
            InterfaceDirection::IspNetwork,
            heimdall_event_handler,
            flowbee_event_handler,
        )?;
        BPF_SKELETON.lock().replace(LqosKernBpfWrapper {
            ptr: skeleton,
            kprobe_link,
        });
        Ok(kernel)
    }

    /// Creates a new `LibreQosKernels` structure, in "on a stick mode" - only a single interface is
    /// bound, and internal VLANs are used to map ingress vs. egress. Returns Ok(self) if everything
    /// loaded correctly, an error otherwise.
    ///
    /// Prints to `stdio` during execution and detailed errors to `stderr`.
    ///
    /// ## Arguments
    ///
    /// * `stick_interfaace` - the name of the VLAN trunked interface.
    /// * `internet_vlan` - the VLAN ID facing the Internet. Endianness is fixed for you.
    /// * `isp_vlan` - the VLAN ID facing the ISP core router. Endianness is fixed for you.
    pub fn on_a_stick_mode<S: ToString>(
        stick_interface: S,
        internet_vlan: u16,
        isp_vlan: u16,
        stick_offset: u32,
        heimdall_event_handler: ring_buffer_sample_fn,
        flowbee_event_handler: ring_buffer_sample_fn,
    ) -> anyhow::Result<Self> {
        let kernel = Self {
            to_internet: stick_interface.to_string(),
            to_isp: String::new(),
            on_a_stick: true,
        };
        let stick_ifindex =
            crate::lqos_kernel::interface_name_to_index(&kernel.to_internet)? as i32;
        let AttachedPrograms {
            skeleton,
            kprobe_link,
        } = attach_xdp_and_tc_to_interface(
            &kernel.to_internet,
            stick_ifindex,
            -1,
            true,
            InterfaceDirection::OnAStick(internet_vlan, isp_vlan, stick_offset),
            heimdall_event_handler,
            flowbee_event_handler,
        )?;
        BPF_SKELETON.lock().replace(LqosKernBpfWrapper {
            ptr: skeleton,
            kprobe_link,
        });
        Ok(kernel)
    }
}

impl Drop for LibreQoSKernels {
    fn drop(&mut self) {
        let skeleton = BPF_SKELETON.lock().take();
        drop(skeleton);
        if !self.on_a_stick {
            let _ = unload_xdp_from_interface(&self.to_internet);
            let _ = unload_xdp_from_interface(&self.to_isp);
        } else {
            let _ = unload_xdp_from_interface(&self.to_internet);
        }
    }
}
