use crate::lqos_kernel::{
    attach_xdp_and_tc_to_interface, unload_xdp_from_interface, InterfaceDirection,
};

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
    pub fn new<S: ToString>(to_internet: S, to_isp: S) -> anyhow::Result<Self> {
        let kernel = Self {
            to_internet: to_internet.to_string(),
            to_isp: to_isp.to_string(),
            on_a_stick: false,
        };
        attach_xdp_and_tc_to_interface(&kernel.to_internet, InterfaceDirection::Internet)?;
        attach_xdp_and_tc_to_interface(&kernel.to_isp, InterfaceDirection::IspNetwork)?;
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
    pub fn on_a_stick_mode<S:ToString>(stick_interface : S, internet_vlan: u16, isp_vlan: u16) -> anyhow::Result<Self> {
        let kernel = Self {
            to_internet: stick_interface.to_string(),
            to_isp: String::new(),
            on_a_stick: true,
        };
        attach_xdp_and_tc_to_interface(&kernel.to_internet, InterfaceDirection::OnAStick(internet_vlan, isp_vlan))?;

        Ok(kernel)
    }
}

impl Drop for LibreQoSKernels {
    fn drop(&mut self) {
        if !self.on_a_stick {
            let _ = unload_xdp_from_interface(&self.to_internet);
            let _ = unload_xdp_from_interface(&self.to_isp);
        } else {
            let _ = unload_xdp_from_interface(&self.to_internet);
        }
    }
}
