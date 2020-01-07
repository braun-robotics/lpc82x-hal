//! API for USART
//!
//! The entry point to this API is [`USART`]. Currently, only some limited UART
//! functionality is implemented.
//!
//! The USART peripheral is described in the user manual, chapter 13.
//!
//! # Examples
//!
//! ``` no_run
//! use lpc82x_hal::prelude::*;
//! use lpc82x_hal::Peripherals;
//! use lpc82x_hal::usart::{
//!     BaudRate,
//!     USART,
//! };
//!
//! let mut p = Peripherals::take().unwrap();
//!
//! let mut syscon = p.SYSCON.split();
//! let mut swm    = p.SWM.split();
//!
//! // Set baud rate to 115200 baud
//! // Please refer to the USART example in the repository for a full
//! // explanation of this value.
//! syscon.uartfrg.set_clkdiv(6);
//! syscon.uartfrg.set_frgmult(22);
//! syscon.uartfrg.set_frgdiv(0xff);
//! let baud_rate = BaudRate::new(&syscon.uartfrg, 0);
//!
//! let (u0_rxd, _) = swm.movable_functions.u0_rxd.assign(
//!     swm.pins.pio0_0.into_swm_pin(),
//!     &mut swm.handle,
//! );
//! let (u0_txd, _) = swm.movable_functions.u0_txd.assign(
//!     swm.pins.pio0_4.into_swm_pin(),
//!     &mut swm.handle,
//! );
//!
//! // Initialize USART0. This should never fail, as the only reason `init`
//! // returns a `Result::Err` is when the transmitter is busy, which it
//! // shouldn't be right now.
//! let mut serial = p.USART0.enable(
//!     &baud_rate,
//!     &mut syscon.handle,
//!     u0_rxd,
//!     u0_txd,
//! );
//!
//! // Use a blocking method to write a string
//! serial.tx().bwrite_all(b"Hello, world!");
//! ```
//!
//! Please refer to the [examples in the repository] for more example code.
//!
//! [examples in the repository]: https://github.com/lpc-rs/lpc8xx-hal/tree/master/lpc82x-hal/examples

use core::fmt;
use core::ops::Deref;

use embedded_hal::blocking::serial::write::Default as BlockingWriteDefault;
use embedded_hal::serial::{Read, Write};
use nb::{self, block};
use void::Void;

use crate::{
    dma, init_state,
    pac::{self, usart0::TXDAT, Interrupt, NVIC},
    swm::{self, FunctionTrait, PinTrait},
    syscon::{self, PeripheralClock},
};

/// Interface to a USART peripheral
///
/// Controls the USART.  Use [`Peripherals`] to gain access to an instance of
/// this struct.
///
/// Please refer to the [module documentation] for more information.
///
/// [`Peripherals`]: ../struct.Peripherals.html
/// [module documentation]: index.html
pub struct USART<UsartX, State = init_state::Enabled> {
    usart: UsartX,
    _state: State,
}

impl<UsartX> USART<UsartX, init_state::Disabled> {
    pub(crate) fn new(usart: UsartX) -> Self {
        USART {
            usart: usart,
            _state: init_state::Disabled,
        }
    }
}

impl<UsartX> USART<UsartX, init_state::Disabled>
where
    UsartX: Peripheral,
{
    /// Enable the USART
    ///
    /// This method is only available, if `USART` is in the [`Disabled`] state.
    /// Code that attempts to call this method when the peripheral is already
    /// enabled will not compile.
    ///
    /// Consumes this instance of `USART` and returns another instance that has
    /// its `State` type parameter set to [`Enabled`].
    ///
    /// # Limitations
    ///
    /// For USART to function correctly, the UARTFRG reset must be cleared. This
    /// is the default, so unless you have messed with those settings, you
    /// should be good.
    ///
    /// # Examples
    ///
    /// Please refer to the [module documentation] for a full example.
    ///
    /// [`Disabled`]: ../init_state/struct.Disabled.html
    /// [`Enabled`]: ../init_state/struct.Enabled.html
    /// [`BaudRate`]: struct.BaudRate.html
    /// [module documentation]: index.html
    pub fn enable<'a, Rx, Tx, CLOCK>(
        mut self,
        clock: &CLOCK,
        syscon: &mut syscon::Handle,
        _: swm::Function<UsartX::Rx, swm::state::Assigned<Rx>>,
        _: swm::Function<UsartX::Tx, swm::state::Assigned<Tx>>,
    ) -> USART<UsartX, init_state::Enabled>
    where
        Rx: PinTrait,
        Tx: PinTrait,
        UsartX::Rx: FunctionTrait<Rx>,
        UsartX::Tx: FunctionTrait<Tx>,
        CLOCK: PeripheralClock<UsartX>,
    {
        syscon.enable_clock(&mut self.usart);

        clock.select_clock(syscon);
        self.usart
            .brg
            .write(|w| unsafe { w.brgval().bits(clock.get_psc()) });

        // According to the user manual, section 13.6.1, we need to make sure
        // that the USART is not sending or receiving data before writing to
        // CFG, and that it is disabled. We statically know that it is disabled
        // at this point, so there isn't anything to do here to ensure it.

        self.usart.cfg.modify(|_, w| {
            w.enable().enabled();
            w.datalen().bit_8();
            w.paritysel().no_parity();
            w.stoplen().bit_1();
            w.ctsen().disabled();
            w.syncen().asynchronous_mode();
            w.loop_().normal();
            w.autoaddr().disabled();
            w.rxpol().standard();
            w.txpol().standard()
        });

        self.usart.ctl.modify(|_, w| {
            w.txbrken().normal();
            w.addrdet().disabled();
            w.txdis().enabled();
            w.autobaud().disabled()
        });

        USART {
            usart: self.usart,
            _state: init_state::Enabled(()),
        }
    }
}

impl<UsartX> USART<UsartX, init_state::Enabled>
where
    UsartX: Peripheral,
{
    /// Disable the USART
    ///
    /// This method is only available, if `USART` is in the [`Enabled`] state.
    /// Code that attempts to call this method when the peripheral is already
    /// disabled will not compile.
    ///
    /// Consumes this instance of `USART` and returns another instance that has
    /// its `State` type parameter set to [`Disabled`].
    ///
    /// [`Enabled`]: ../init_state/struct.Enabled.html
    /// [`Disabled`]: ../init_state/struct.Disabled.html
    pub fn disable(mut self, syscon: &mut syscon::Handle) -> USART<UsartX, init_state::Disabled> {
        syscon.disable_clock(&mut self.usart);

        USART {
            usart: self.usart,
            _state: init_state::Disabled,
        }
    }

    /// Enable the USART interrupts
    ///
    /// Enable the interrupts for this USART peripheral. This only enables the
    /// interrupts via the NVIC. It doesn't enable any specific interrupt.
    pub fn enable_interrupts(&mut self) {
        // Safe, because there's no critical section here that this could
        // interfere with.
        unsafe { NVIC::unmask(UsartX::INTERRUPT) };
    }

    /// Return USART receiver
    pub fn rx(&self) -> Receiver<UsartX> {
        Receiver(self)
    }

    /// Return USART transmitter
    pub fn tx(&self) -> Transmitter<UsartX> {
        Transmitter(self)
    }
}

impl<UsartX, State> USART<UsartX, State> {
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
    pub fn free(self) -> UsartX {
        self.usart
    }
}

/// USART receiver
pub struct Receiver<'usart, UsartX: 'usart>(&'usart USART<UsartX>);

impl<'usart, UsartX> Receiver<'usart, UsartX>
where
    UsartX: Peripheral,
{
    /// Enable the RXRDY interrupt
    ///
    /// The interrupt will not actually work unless the interrupts for this
    /// peripheral have also been enabled via the NVIC. See
    /// [`enable_interrupts`].
    ///
    /// [`enable_interrupts`]: #method.enable_interrupts
    pub fn enable_rxrdy_interrupt(&mut self) {
        self.0.usart.intenset.write(|w| w.rxrdyen().set_bit());
    }

    /// Disable the RXRDY interrupt
    pub fn disable_rxrdy_interrupt(&mut self) {
        self.0.usart.intenclr.write(|w| w.rxrdyclr().set_bit());
    }
}

impl<'usart, UsartX> Read<u8> for Receiver<'usart, UsartX>
where
    UsartX: Peripheral,
{
    type Error = Error;

    fn read(&mut self) -> nb::Result<u8, Self::Error> {
        let stat = self.0.usart.stat.read();

        if stat.rxbrk().bit_is_set() {
            return Err(nb::Error::WouldBlock);
        }
        // TODO Due to SVD bug not available
        #[cfg(feature = "82x")]
        {
            if stat.overrunint().bit_is_set() {
                return Err(nb::Error::Other(Error::Overrun));
            }
        }

        if stat.rxrdy().bit_is_set() {
            // It's important to read this register all at once, as reading
            // it changes the status flags.
            let rx_dat_stat = self.0.usart.rxdatstat.read();

            if rx_dat_stat.framerr().bit_is_set() {
                return Err(nb::Error::Other(Error::Framing));
            }
            if rx_dat_stat.parityerr().bit_is_set() {
                return Err(nb::Error::Other(Error::Parity));
            }
            if rx_dat_stat.rxnoise().bit_is_set() {
                return Err(nb::Error::Other(Error::Noise));
            }

            // `bits` returns `u16`, but at most 9 bits are used. We've
            // configured UART to use only 8 bits, so we can safely cast to
            // `u8`.
            return Ok(rx_dat_stat.rxdat().bits() as u8);
        } else {
            return Err(nb::Error::WouldBlock);
        }
    }
}

/// USART transmitter
pub struct Transmitter<'usart, UsartX: 'usart>(&'usart USART<UsartX>);

impl<'usart, UsartX> Transmitter<'usart, UsartX>
where
    UsartX: Peripheral,
{
    /// Enable the TXRDY interrupt
    ///
    /// The interrupt will not actually work unless the interrupts for this
    /// peripheral have also been enabled via the NVIC. See
    /// [`enable_interrupts`].
    ///
    /// [`enable_interrupts`]: #method.enable_interrupts
    pub fn enable_txrdy_interrupt(&mut self) {
        self.0.usart.intenset.write(|w| w.txrdyen().set_bit());
    }

    /// Disable the TXRDY interrupt
    pub fn disable_txrdy_interrupt(&mut self) {
        self.0.usart.intenclr.write(|w| w.txrdyclr().set_bit());
    }
}

impl<'usart, UsartX> Write<u8> for Transmitter<'usart, UsartX>
where
    UsartX: Peripheral,
{
    type Error = Void;

    fn write(&mut self, word: u8) -> nb::Result<(), Self::Error> {
        if self.0.usart.stat.read().txrdy().bit_is_clear() {
            return Err(nb::Error::WouldBlock);
        }

        unsafe {
            self.0.usart.txdat.write(|w| w.txdat().bits(word as u16));
        }

        Ok(())
    }

    fn flush(&mut self) -> nb::Result<(), Self::Error> {
        if self.0.usart.stat.read().txidle().bit_is_clear() {
            return Err(nb::Error::WouldBlock);
        }

        Ok(())
    }
}

impl<'usart, UsartX> BlockingWriteDefault<u8> for Transmitter<'usart, UsartX> where
    UsartX: Peripheral
{
}

impl<'usart, UsartX> fmt::Write for Transmitter<'usart, UsartX>
where
    Self: BlockingWriteDefault<u8>,
    UsartX: Peripheral,
{
    fn write_str(&mut self, s: &str) -> fmt::Result {
        use crate::prelude::*;

        self.bwrite_all(s.as_bytes()).map_err(|_| fmt::Error)?;
        block!(self.flush()).map_err(|_| fmt::Error)?;

        Ok(())
    }
}

impl<'usart, UsartX> dma::Dest for Transmitter<'usart, UsartX>
where
    UsartX: Peripheral,
{
    type Error = Void;

    fn wait(&mut self) -> nb::Result<(), Self::Error> {
        self.flush()
    }

    fn end_addr(&mut self) -> *mut u8 {
        &self.0.usart.txdat as *const _ as *mut TXDAT as *mut u8
    }
}

/// Internal trait for USART peripherals
///
/// This trait is an internal implementation detail and should neither be
/// implemented nor used outside of LPC82x HAL. Any changes to this trait won't
/// be considered breaking changes.
pub trait Peripheral:
    Deref<Target = pac::usart0::RegisterBlock> + syscon::ClockControl + syscon::ResetControl
{
    /// The interrupt that is triggered for this USART peripheral
    const INTERRUPT: Interrupt;

    /// The movable function that needs to be assigned to this USART's RX pin
    type Rx;

    /// The movable function that needs to be assigned to this USART's TX pin
    type Tx;
}

impl Peripheral for pac::USART0 {
    const INTERRUPT: Interrupt = Interrupt::USART0;

    type Rx = swm::U0_RXD;
    type Tx = swm::U0_TXD;
}

impl Peripheral for pac::USART1 {
    const INTERRUPT: Interrupt = Interrupt::USART1;

    type Rx = swm::U1_RXD;
    type Tx = swm::U1_TXD;
}

impl Peripheral for pac::USART2 {
    const INTERRUPT: Interrupt = Interrupt::USART2;

    type Rx = swm::U2_RXD;
    type Tx = swm::U2_TXD;
}

#[cfg(feature = "845")]
impl Peripheral for pac::USART3 {
    /// Since we don't provide an abstraction for pin interrupts,
    /// it's alright to ignore the shared interrupt
    const INTERRUPT: Interrupt = Interrupt::PIN_INT6_USART3;

    type Rx = swm::U3_RXD;
    type Tx = swm::U3_TXD;
}

#[cfg(feature = "845")]
impl Peripheral for pac::USART4 {
    const INTERRUPT: Interrupt = Interrupt::PIN_INT7_USART4;

    type Rx = swm::U4_RXD;
    type Tx = swm::U4_TXD;
}

/// A USART error
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Character received with a stop bit missing at the expected location
    Framing,

    /// Corrupted character received
    Noise,

    /// Character received, while receiver buffer was still in use
    Overrun,

    /// Parity error detected in received character
    Parity,
}
