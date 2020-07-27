use crate::{init_state, pac, syscon};

use super::Channels;

/// Entry point to the DMA API
pub struct DMA<State> {
    dma: pac::DMA0,
    srambase: u32,

    /// The DMA channels
    pub channels: Channels<State>,
}

impl DMA<init_state::Disabled> {
    pub(crate) fn new(dma: pac::DMA0) -> Self {
        let descriptors = unsafe { &mut super::descriptors::DESCRIPTORS };
        let srambase = descriptors as *mut _ as u32;

        Self {
            dma,
            srambase,
            channels: Channels::new(descriptors),
        }
    }

    /// Enable the DMA controller
    pub fn enable(
        self,
        syscon: &mut syscon::Handle,
    ) -> DMA<init_state::Enabled> {
        syscon.enable_clock(&self.dma);

        // Set descriptor table address
        //
        // See user manual, section 12.6.3.
        self.dma
            .srambase
            .write(|w| unsafe { w.bits(self.srambase) });

        // Enable the DMA controller
        //
        // See user manual, section 12.6.1.
        self.dma.ctrl.write(|w| w.enable().enabled());

        DMA {
            dma: self.dma,
            srambase: self.srambase,
            channels: self.channels.enable(),
        }
    }
}

impl DMA<init_state::Enabled> {
    /// Disable the DMA controller
    pub fn disable(
        self,
        syscon: &mut syscon::Handle,
    ) -> DMA<init_state::Disabled> {
        syscon.disable_clock(&self.dma);

        DMA {
            dma: self.dma,
            srambase: self.srambase,
            channels: self.channels.disable(),
        }
    }
}

impl<State> DMA<State> {
    /// Return the raw peripheral
    ///
    /// This method serves as an escape hatch from the HAL API. It returns the
    /// raw peripheral, allowing you to do whatever you want with it, without
    /// limitations imposed by the API.
    ///
    /// If you are using this method because a feature you need is missing from
    /// the HAL API, please [open an issue] or, if an issue for your feature
    /// request already exists, comment on the existing issue, so we can
    /// prioritize it accordingly.
    ///
    /// [open an issue]: https://github.com/lpc-rs/lpc8xx-hal/issues
    pub fn free(self) -> pac::DMA0 {
        self.dma
    }
}
