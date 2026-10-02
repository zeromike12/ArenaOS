//! Parse the kernel's frozen 12-word `SYS_DEV_INFO` record for type-16 GPU.
//!
//! Parsing is not device authority. The kernel must have checked that the
//! caller holds a covering MMIO cap, and a future displayd must map that
//! very cap before accessing any `Window`. No raw BAR base from this record
//! is turned into a user mapping or a DMA authorization here.

pub const WORDS: usize = 12;
pub const MODERN_GPU_ID: u64 = 0x1050;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoError {
    Device,
    Layout,
    Overflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub pci_index: u64,
    pub common_offset: u32,
    pub common_length: u32,
    pub notify_offset: u32,
    pub notify_length: u32,
    pub notify_multiplier: u32,
    pub bar_base: u64,
}

fn pair(word: u64) -> (u32, u32) {
    (word as u32, (word >> 32) as u32)
}
fn span(word: u64, minimum: u32) -> Result<(u32, u32), InfoError> {
    let (off, len) = pair(word);
    if len < minimum || off.checked_add(len).is_none() {
        return Err(InfoError::Overflow);
    }
    Ok((off, len))
}
impl DeviceInfo {
    /// These bytes came from `SYS_DEV_INFO` for a device index authorized
    /// by a held cap. The frozen ABI leaves BAR size out; **do not deref**
    /// offsets until the kernel's separate held-cap/BAR coverage check is
    /// installed and its negative tests have passed (ADR-0058).
    pub fn parse(w: &[u64; WORDS]) -> Result<Self, InfoError> {
        if (w[8] & 0xffff) != MODERN_GPU_ID || w[8] >> 16 != 0 || w[9..].iter().any(|&x| x != 0) {
            return Err(InfoError::Device);
        }
        if w[1] == 0 || !w[1].is_multiple_of(4096) || w[4] == 0 || w[4] > u32::MAX as u64 {
            return Err(InfoError::Layout);
        }
        let (common_offset, common_length) = span(w[2], 0x38)?;
        let (notify_offset, notify_length) = span(w[3], 2)?;
        // The device-info gate historically requires all four capabilities
        // on the same BAR. A minimal ISR/device cfg pair is still required
        // even though a polled 2D driver does not access them.
        let _isr = span(w[5], 1)?;
        let _devcfg = span(w[6], 1)?;
        Ok(Self {
            pci_index: w[0],
            bar_base: w[1],
            common_offset,
            common_length,
            notify_offset,
            notify_length,
            notify_multiplier: w[4] as u32,
        })
    }
    /// Address arithmetic only; not a proof that memory is mapped or
    /// actually belongs to this device (the cap gate must supply that).
    pub fn window(self, mapped_base: u64) -> Result<(u64, u64), InfoError> {
        if mapped_base == 0 {
            return Err(InfoError::Layout);
        }
        let common_end = u64::from(self.common_offset) + u64::from(self.common_length);
        let notify_end = u64::from(self.notify_offset) + u64::from(self.notify_length);
        mapped_base
            .checked_add(common_end)
            .ok_or(InfoError::Overflow)?;
        mapped_base
            .checked_add(notify_end)
            .ok_or(InfoError::Overflow)?;
        let common = mapped_base + u64::from(self.common_offset);
        let notify = mapped_base + u64::from(self.notify_offset);
        Ok((common, notify))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn good() -> [u64; WORDS] {
        let mut w = [0u64; WORDS];
        w[0] = 7;
        w[1] = 0xf000_0000;
        w[2] = 0x38u64 << 32;
        w[3] = (4u64 << 32) | 0x100;
        w[4] = 4;
        w[5] = (1u64 << 32) | 0x200;
        w[6] = (4u64 << 32) | 0x204;
        w[7] = 1u64 | (2u64 << 32); // MSI-X is optional for polling
        w[8] = MODERN_GPU_ID;
        w
    }
    #[test]
    fn exact_modern_gpu_record_and_window_offsets() {
        let d = DeviceInfo::parse(&good()).unwrap();
        assert_eq!(d.pci_index, 7);
        assert_eq!(d.window(0x1000_0000), Ok((0x1000_0000, 0x1000_0100)));
        assert_eq!(d.notify_multiplier, 4);
    }
    #[test]
    fn wrong_type_split_layout_and_offset_overflow_refuse() {
        let mut w = good();
        w[8] = 0x1042;
        assert_eq!(DeviceInfo::parse(&w), Err(InfoError::Device));
        w = good();
        w[9] = 1;
        assert_eq!(DeviceInfo::parse(&w), Err(InfoError::Device));
        w = good();
        w[2] = (0x38u64 << 32) | 0xffff_fff0;
        assert_eq!(DeviceInfo::parse(&w), Err(InfoError::Overflow));
        w = good();
        w[4] = 0;
        assert_eq!(DeviceInfo::parse(&w), Err(InfoError::Layout));
        w = good();
        w[3] = 1u64 << 32;
        assert_eq!(DeviceInfo::parse(&w), Err(InfoError::Overflow));
        let d = DeviceInfo::parse(&good()).unwrap();
        assert_eq!(d.window(u64::MAX), Err(InfoError::Overflow));
    }
}
