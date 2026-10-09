//! Erratum 843419: A load or store might access an incorrect address.
//! Software Developers Errata Notice: https://documentation-service.arm.com/static/5fa29fddb209f547eebd361d
//!
//! Description:
//! When executing in AArch64 state, a load or store instruction which uses the result of an ADRP
//! instruction as a base register, or which uses a base register written by an instruction
//! immediately after an ADRP to the same register, might access an incorrect address.
//!
//! Workaround: Prevent affected sequences from crossing a 4 KiB page boundary by keeping
//! the ADRP away from page offsets 0xFF8 and 0xFFC.
//!
//! Variant 2 is intentionally excluded because it involves a dead ADRP instruction:
//! where the following instruction overwrites its destination register.

use crate::Result;
use crate::elf_aarch64::MIN_BRANCH_RANGE;
use crate::ensure;
use hashbrown::HashMap;
use smallvec::SmallVec;

const INSN_SIZE: usize = 4;

const ADRP_MARK: u32 = 0x9f00_0000;
const ADRP_OPCODE: u32 = 0x9000_0000;
// LDR (unsigned offset)
const LDR_UNSIGNED_MASK: u32 = 0xffc0_0000;
const LDR_UNSIGNED_OPCODE: u32 = 0xf940_0000;
// ADD (immediate)
const ADD_IMM_MASK: u32 = 0xffc0_0000;
const ADD_IMM_OPCODE: u32 = 0x9100_0000;
// Load/store register (unsigned immediate) (ARM Manual category of instructions)
const LDR_STR_UNSIGNED_MASK: u32 = 0x3b00_0000;
const LDR_STR_UNSIGNED_OPCODE: u32 = 0x3900_0000;
const B_OPCODE: u32 = 0x1400_0000;
const NOP_OPCODE: u32 = 0xd503201f;

const REGISTER_MASK: u32 = (1 << 5) - 1;

#[derive(Debug)]
enum ArmInsn {
    Adrp { rd: u32 },
    Branch,
    Ldr { rt: u32, rn: u32 },
    Add { rd: u32 },
    LdrStr { rn: u32 },
    Unrecognized,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ErratumVariant {
    // Sequence 1 with 4 instructions
    Sequence1A,
    // Sequence 1 with 3 instructions
    Sequence1B,
}

/// An erratum variant and its section-relative ADRP byte offset.
pub(crate) type ErratumOffset = (ErratumVariant, usize);

impl ErratumVariant {
    fn instruction_count(self) -> usize {
        match self {
            Self::Sequence1A => 4,
            Self::Sequence1B => 3,
        }
    }
}

impl ArmInsn {
    fn is_final_load_store_imm(insn: &ArmInsn, register: u32) -> bool {
        match insn {
            Self::LdrStr { rn, .. } if *rn == register => true,
            Self::Ldr { rn, .. } if *rn == register => true,
            _ => false,
        }
    }

    fn is_branch(insn: u32) -> bool {
        // B, BL
        (insn & 0x7c000000) == B_OPCODE
           // B.cond and BC.cond
           || (insn & 0xff000000) == 0x54000000
           // CBZ, CBNZ
           || (insn & 0x7e000000) == 0x34000000
           // TBZ, TBNZ
           || (insn & 0x7e000000) == 0x36000000
           // BR, BLR, RET and authenticated/register control transfers
           || (insn & 0xfe000000) == 0xd6000000
    }

    fn from_opcode(insn: u32) -> Self {
        if insn & ADRP_MARK == ADRP_OPCODE {
            Self::Adrp {
                rd: insn & REGISTER_MASK,
            }
        } else if insn & LDR_UNSIGNED_MASK == LDR_UNSIGNED_OPCODE {
            // Ldr is a subset of LdrStr, so decode it first.
            Self::Ldr {
                rt: insn & REGISTER_MASK,
                rn: (insn >> 5) & REGISTER_MASK,
            }
        } else if insn & ADD_IMM_MASK == ADD_IMM_OPCODE {
            Self::Add {
                rd: insn & REGISTER_MASK,
            }
        } else if insn & LDR_STR_UNSIGNED_MASK == LDR_STR_UNSIGNED_OPCODE {
            Self::LdrStr {
                // The target register (rt) is not needed here.
                rn: (insn >> 5) & REGISTER_MASK,
            }
        } else if Self::is_branch(insn) {
            Self::Branch
        } else {
            Self::Unrecognized
        }
    }

    /// Returns true if the instruction sequence begins with an ADRP that could trigger
    /// erratum 843419.
    fn starts_with_erratum_843419(insns: &[[u8; INSN_SIZE]]) -> Option<ErratumVariant> {
        let get_insn = |data| Self::from_opcode(u32::from_le_bytes(data));

        // Apparently the following `let Adrp` is not optimized so ideally as the following check!
        if (u32::from_le_bytes(insns[0])) & ADRP_MARK != ADRP_OPCODE {
            return None;
        }

        // 1) ADRP
        let ArmInsn::Adrp { rd: register } = get_insn(insns[0]) else {
            return None;
        };

        // 2) The next instruction must not overwrite the ADRP destination register.
        // This must not write to Rn.
        match get_insn(insns[1]) {
            Self::Add { .. } | Self::Adrp { .. } | Self::Branch => return None,
            ArmInsn::Ldr { rt, .. } if rt == register => return None,
            _ => {}
        }

        let third = get_insn(insns[2]);
        // 3) Variant A (optional 3rd instruction)
        if insns.len() >= 4 {
            // This cannot be a branch.
            // This cannot write Rn.
            match third {
                Self::Branch => {}
                Self::Add { rd, .. } if rd == register => {}
                Self::Ldr { rt, .. } if rt == register => {}
                ArmInsn::Adrp { rd } if rd == register => {}
                _ => {
                    // 4) Load/store register (unsigned immediate)" encoding class, using Rn as the
                    //    base address register.
                    if Self::is_final_load_store_imm(&get_insn(insns[3]), register) {
                        return Some(ErratumVariant::Sequence1A);
                    }
                }
            }
        }

        // 3) Variant B
        Self::is_final_load_store_imm(&third, register)
            .then_some(ErratumVariant::Sequence1B)
            .or(None)
    }
}

/// Returns byte offsets of all potential erratum sequences in the instruction stream.
pub(crate) fn erratum_843419_offsets(data: &[u8]) -> SmallVec<[ErratumOffset; 2]> {
    let insns = data.as_chunks::<INSN_SIZE>().0;

    // A busy loop where we intentionally use a plain loop as the Iterator abstraction
    // is not zero cost.
    let mut result = SmallVec::new();
    for index in 0..insns.len().saturating_sub(2) {
        if let Some(variant) = ArmInsn::starts_with_erratum_843419(&insns[index..]) {
            result.push((variant, index * INSN_SIZE));
        }
    }
    result
}

/// The erratum depends on the low 12 bits of the instruction address.
const ERRATUM_PAGE_SIZE: usize = 4096;
/// The number of instruction positions in one erratum page.
const ERRATUM_INSN_OFFSETS: usize = ERRATUM_PAGE_SIZE / INSN_SIZE;

#[derive(Debug)]
pub(crate) struct ErratumSectionInfo {
    pub(crate) offsets: SmallVec<[ErratumOffset; 2]>,
    // Maximum padding in bytes needed to make any valid section placement safe.
    pub(crate) maximal_padding: usize,
}

/// Returns whether an ADRP offset, in instruction units, is safe from the erratum.
fn is_safe_adrp_offset(offset: usize) -> bool {
    let page_insn_offset = offset % ERRATUM_INSN_OFFSETS;
    page_insn_offset != 0xff8 / INSN_SIZE && page_insn_offset != 0xffc / INSN_SIZE
}

fn erratum_mask_from_offset(
    erratum_offsets: &[ErratumOffset],
    section_alignment: u64,
    section_size: usize,
) -> Result<Option<ErratumSectionInfo>> {
    let section_alignment = section_alignment as usize;
    if erratum_offsets.is_empty() {
        return Ok(None);
    }

    ensure!(
        section_alignment >= INSN_SIZE,
        "unexpected small alignment: {section_alignment}"
    );
    let mut maximal_padding = 0;

    // Take the worst case over every instruction-aligned page-relative section start,
    // independently of the section's alignment or its eventual placement.
    for section_start in 0..ERRATUM_INSN_OFFSETS {
        let mut tail = section_size.div_ceil(INSN_SIZE);
        for (variant, _) in erratum_offsets
            .iter()
            .filter(|(_, offset)| !is_safe_adrp_offset(*offset / INSN_SIZE + section_start))
        {
            while !is_safe_adrp_offset(section_start + tail) {
                // Keep the relocated ADRP away from unsafe page offsets, including for later stubs.
                tail += 1;
            }
            tail += variant.instruction_count() + 1;
        }
        maximal_padding = maximal_padding.max(tail * INSN_SIZE - section_size);
    }

    ensure!(
        (section_size + maximal_padding) as u64 <= MIN_BRANCH_RANGE,
        "Input section too large for erratum workaround code"
    );

    Ok(Some(ErratumSectionInfo {
        offsets: SmallVec::from_slice(erratum_offsets),
        maximal_padding,
    }))
}

/// Copies affected sequences into reserved tail slots and maps each moved instruction's address.
pub(crate) fn patch_erratum_sequences(
    out: &mut [u8],
    section_size: usize,
    section_address: u64,
    offsets: &[ErratumOffset],
) -> Result<HashMap<u64, u64>> {
    let mut mapping = HashMap::new();
    let mut tail = section_size;
    for &(variant, offset) in offsets {
        if is_safe_adrp_offset((section_address as usize + offset) / INSN_SIZE) {
            continue;
        }
        while !is_safe_adrp_offset((section_address as usize + tail) / INSN_SIZE) {
            tail += INSN_SIZE;
        }
        let size = variant.instruction_count() * INSN_SIZE;
        ensure!(
            offset + size <= section_size,
            "Erratum sequence exceeds section size"
        );
        ensure!(
            tail + size + INSN_SIZE <= out.len(),
            "Insufficient space for erratum slot"
        );
        // Copy erratum instructions to the tail slot.
        out.copy_within(offset..offset + size, tail);
        for instruction in (0..size).step_by(INSN_SIZE) {
            mapping.insert(
                section_address + (offset + instruction) as u64,
                section_address + (tail + instruction) as u64,
            );
        }
        // BR(veneer), NOP, NOP, [NOP]
        write_branch(out, offset, tail)?;
        for instruction in (offset + INSN_SIZE..offset + size).step_by(INSN_SIZE) {
            out[instruction..instruction + INSN_SIZE].copy_from_slice(&NOP_OPCODE.to_le_bytes());
        }
        // BR(back after NOPS)
        write_branch(out, tail + size, offset + size)?;
        tail += size + INSN_SIZE;
    }
    Ok(mapping)
}

fn write_branch(out: &mut [u8], from: usize, to: usize) -> Result {
    let displacement = u32::try_from(to - from)?;
    out[from..from + INSN_SIZE]
        .copy_from_slice(&(B_OPCODE | (displacement / INSN_SIZE as u32)).to_le_bytes());
    Ok(())
}

pub(crate) fn erratum_section_info(
    data: &[u8],
    section_alignment: u64,
) -> Result<Option<ErratumSectionInfo>> {
    debug_assert!(section_alignment.is_power_of_two());
    let erratum_offsets = erratum_843419_offsets(data);
    erratum_mask_from_offset(&erratum_offsets, section_alignment, data.len())
}
