//! Builds a Mach-O compact unwind section.

use crate::ensure;
use crate::error::Result;
use crate::macho::ResolvedUnwindInfo;
use crate::macho::UnwindInfoWithRelocs;
use anyhow::Context;
use hashbrown::HashMap;
use itertools::Itertools;
use zerocopy::FromBytes;
use zerocopy::Immutable;
use zerocopy::IntoBytes;

// TODO: taken from macho-unwind-info crate

// Written with help from https://gankra.github.io/blah/compact-unwinding/

/// The `__unwind_info` header.
#[derive(FromBytes, IntoBytes, Immutable, Debug, Clone, Copy)]
#[repr(C)]
pub struct CompactUnwindInfoHeader {
    /// The version. Only version 1 is currently defined
    pub version: u32,

    /// The array of U32 global opcodes (offset relative to start of root page).
    ///
    /// These may be indexed by "compressed" second-level pages.
    pub global_opcodes_offset: u32,
    pub global_opcodes_len: u32,

    /// The array of U32 global personality codes (offset relative to start of root page).
    ///
    /// Personalities define the style of unwinding that an unwinder should use,
    /// and how to interpret the LSDA functions for a function (see below).
    pub personalities_offset: u32,
    pub personalities_len: u32,

    /// The array of [`PageEntry`]'s describing the second-level pages
    /// (offset relative to start of root page).
    pub pages_offset: u32,
    pub pages_len: u32,
    // After this point there are several dynamically-sized arrays whose precise
    // order and positioning don't matter, because they are all accessed using
    // offsets like the ones above. The arrays are:

    // global_opcodes: [u32; global_opcodes_len],
    // personalities: [u32; personalities_len],
    // pages: [PageEntry; pages_len],
    // lsdas: [LsdaEntry; unknown_len],
}

/// One element of the array of pages.
#[derive(FromBytes, IntoBytes, Clone, Copy)]
#[repr(C)]
pub struct PageEntry {
    /// The first address mapped by this page.
    ///
    /// This is useful for binary-searching for the page that can map
    /// a specific address in the binary (the primary kind of lookup
    /// performed by an unwinder).
    pub first_address: u32,

    /// Offset of the second-level page.
    ///
    /// This may point to either a [`RegularPage`] or a [`CompressedPage`].
    /// Which it is can be determined by the 32-bit "kind" value that is at
    /// the start of both layouts.
    pub page_offset: u32,

    /// Base offset into the lsdas array that functions in this page will be
    /// relative to.
    pub lsda_index_offset: u32,
}

/// A "compressed" page.
#[derive(FromBytes, IntoBytes, Debug, Clone, Copy)]
#[repr(C)]
pub struct CompressedPage {
    /// Always 3 (use to distinguish from RegularPage).
    pub kind: u32,

    /// The array of compressed u32 function entries (offset relative to **start of this page**).
    ///
    /// Entries are a u32 that contains two packed values (from highest to lowest bits):
    /// * 8 bits: opcode index
    ///   * 0..global_opcodes_len => index into global palette
    ///   * global_opcodes_len..255 => index into local palette (subtract global_opcodes_len)
    /// * 24 bits: instruction address
    ///   * address is relative to this page's first_address!
    pub functions_offset: u16,
    pub functions_len: u16,

    /// The array of u32 local opcodes for this page (offset relative to **start of this page**).
    pub local_opcodes_offset: u16,
    pub local_opcodes_len: u16,
}

const COMPRESSED_PAGE_SIZE: usize = 4096;
const COMPRESED_PAGE_ENTRIES_COUNT: usize =
    (COMPRESSED_PAGE_SIZE - size_of::<CompressedPage>()) / size_of::<u32>();

pub(crate) fn output_size(unwind_info_entries: &[UnwindInfoWithRelocs]) -> Result<u64> {
    let encoding_values = unwind_info_entries
        .iter()
        .map(|entry| entry.entry.encoding)
        .unique()
        .count();
    // TODO: relax
    ensure!(
        u8::try_from(encoding_values).is_ok(),
        "too many encodings in compact unwind: {encoding_values}"
    );

    let personality_fns = unwind_info_entries
        .iter()
        .filter_map(|entry| entry.personality)
        .unique()
        .count();
    ensure!(
        personality_fns <= 4,
        "too many personality functions in the compact unwind: {personality_fns}"
    );

    let compressed_pages = unwind_info_entries
        .len()
        .div_ceil(COMPRESED_PAGE_ENTRIES_COUNT);
    let compressed_pages_total_size = compressed_pages * size_of::<CompressedPage>()
        + unwind_info_entries.len() * size_of::<u32>();
    // PageEntry:CompressedPage mapping is 1:1
    let page_entries_total_size = compressed_pages * size_of::<PageEntry>();
    let root_total_size = size_of::<CompactUnwindInfoHeader>()
        + encoding_values * size_of::<u32>()
        + personality_fns * (size_of::<u32>() + size_of::<u64>());

    Ok((compressed_pages_total_size + page_entries_total_size + root_total_size) as u64)
}

pub(crate) fn build(unwind_info_entries: &[ResolvedUnwindInfo]) -> Result<Vec<u8>> {
    let encoding_values = unwind_info_entries
        .iter()
        .map(|entry| entry.entry.encoding)
        .collect_vec();
    dbg!(&encoding_values);
    let encoding_to_index: HashMap<u32, usize> = HashMap::from_iter(
        encoding_values
            .iter()
            .enumerate()
            .map(|(i, encoding)| (*encoding, i)),
    );

    // TODO
    let mut out = Vec::new();

    let off = size_of::<CompactUnwindInfoHeader>() + encoding_values.as_bytes().len();
    let header = CompactUnwindInfoHeader {
        version: 1,
        global_opcodes_offset: size_of::<CompactUnwindInfoHeader>() as u32,
        global_opcodes_len: u32::try_from(encoding_values.len()).unwrap(),
        pages_offset: off as u32,
        pages_len: 0,
        personalities_offset: off as u32,
        personalities_len: 0,
    };
    out.extend(header.as_bytes());
    out.extend(encoding_values.as_bytes());
    dbg!(out.len());

    Ok(out)
}
