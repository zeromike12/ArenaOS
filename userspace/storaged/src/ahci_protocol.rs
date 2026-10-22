//! Pure encoders for the AHCI commands used by the VirtualBox profile.
//! Keeping command construction independent of MMIO makes the on-wire FIS,
//! PRDT bounds, and durable-write command testable on the host.

pub const SECTOR_BYTES: u64 = 512;
pub const FRAME_BYTES: u64 = 4096;
pub const ATA_READ_DMA_EXT: u8 = 0x25;
pub const ATA_WRITE_DMA_EXT: u8 = 0x35;
pub const ATA_FLUSH_CACHE_EXT: u8 = 0xEA;
pub const FIS_REG_H2D: u8 = 0x27;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub command: [u8; 20],
    pub header_flags: u16,
    pub buffer_phys: u64,
    pub bytes: u64,
}

/// Encode a one-sector or one-page ATA DMA EXT request. The caller validates
/// the ArenaOS operation code before selecting the `write` flag.
pub fn encode_transfer(lba: u64, buffer_phys: u64, bytes: u64, write: bool) -> Option<Transfer> {
    let sectors = match bytes {
        SECTOR_BYTES => 1u16,
        FRAME_BYTES => 8u16,
        _ => return None,
    };
    if lba >= (1u64 << 48)
        || lba.checked_add(u64::from(sectors))? > (1u64 << 48)
        || buffer_phys.checked_add(bytes - 1).is_none()
    {
        return None;
    }
    let mut command = [0u8; 20];
    command[0] = FIS_REG_H2D;
    command[1] = 1 << 7; // command FIS
    command[2] = if write {
        ATA_WRITE_DMA_EXT
    } else {
        ATA_READ_DMA_EXT
    };
    command[4] = lba as u8;
    command[5] = (lba >> 8) as u8;
    command[6] = (lba >> 16) as u8;
    command[7] = 1 << 6; // LBA mode
    command[8] = (lba >> 24) as u8;
    command[9] = (lba >> 32) as u8;
    command[10] = (lba >> 40) as u8;
    command[12] = sectors as u8;
    command[13] = (sectors >> 8) as u8;
    Some(Transfer {
        command,
        header_flags: 5 | if write { 1 << 6 } else { 0 },
        buffer_phys,
        bytes,
    })
}

/// Encode ATA FLUSH CACHE EXT. Writes are not acknowledged to the filesystem
/// until this command completes, preserving the synchronous durability
/// contract expected by AFS2's commit protocol.
pub fn encode_flush() -> [u8; 20] {
    let mut command = [0u8; 20];
    command[0] = FIS_REG_H2D;
    command[1] = 1 << 7;
    command[2] = ATA_FLUSH_CACHE_EXT;
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_and_write_dma_ext_encode_lba_and_sector_count() {
        let lba = 0x12_3456_789ABC;
        let read = encode_transfer(lba, 0x2000, SECTOR_BYTES, false).unwrap();
        assert_eq!(read.command[0..3], [FIS_REG_H2D, 0x80, ATA_READ_DMA_EXT]);
        assert_eq!(
            [
                read.command[4],
                read.command[5],
                read.command[6],
                read.command[8],
                read.command[9],
                read.command[10],
            ],
            [0xBC, 0x9A, 0x78, 0x56, 0x34, 0x12]
        );
        assert_eq!(read.command[7], 0x40);
        assert_eq!(&read.command[12..14], &[1, 0]);
        assert_eq!(read.header_flags, 5);

        let write = encode_transfer(lba, 0x3000, FRAME_BYTES, true).unwrap();
        assert_eq!(write.command[2], ATA_WRITE_DMA_EXT);
        assert_eq!(&write.command[12..14], &[8, 0]);
        assert_eq!(write.header_flags, 5 | (1 << 6));
        assert_eq!((write.buffer_phys, write.bytes), (0x3000, FRAME_BYTES));
    }

    #[test]
    fn rejects_unsupported_lengths_lba_overflow_and_buffer_overflow() {
        assert!(encode_transfer(0, 0, 0, false).is_none());
        assert!(encode_transfer(0, 0, 1024, false).is_none());
        assert!(encode_transfer(1 << 48, 0, 512, false).is_none());
        assert!(encode_transfer((1 << 48) - 1, 0, 1024, false).is_none());
        assert!(encode_transfer(0, u64::MAX - 127, 512, false).is_none());
    }

    #[test]
    fn flush_cache_ext_has_no_data_payload() {
        let command = encode_flush();
        assert_eq!(&command[0..3], &[FIS_REG_H2D, 0x80, ATA_FLUSH_CACHE_EXT]);
        assert!(command[3..].iter().all(|byte| *byte == 0));
    }
}
