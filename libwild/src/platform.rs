use crate::arch::Architecture;
use crate::elf::File;
use crate::error::Result;
use crate::macho::MachOFile;
use crate::parsing::DynamicTagValues;

pub(crate) trait ObjectFile<'data>: Send + Sync + Sized {
    fn parse(data: &'data [u8], is_dynamic: bool) -> Result<Self>;

    fn arch(&self) -> Architecture;

    fn dynamic_tag_values(&self) -> Option<DynamicTagValues>;

    fn symbol_count(&self) -> usize;
}

impl<'data> ObjectFile<'data> for File<'data> {
    fn parse(data: &'data [u8], is_dynamic: bool) -> Result<Self> {
        File::parse(data, is_dynamic)
    }

    fn arch(&self) -> Architecture {
        self.arch
    }

    fn dynamic_tag_values(&self) -> Option<DynamicTagValues> {
        self.dynamic_tag_values
    }

    fn symbol_count(&self) -> usize {
        self.symbols.len()
    }
}

impl<'data> ObjectFile<'data> for MachOFile<'data> {
    fn parse(data: &'data [u8], _is_dynamic: bool) -> Result<Self> {
        MachOFile::parse(data)
    }

    fn arch(&self) -> Architecture {
        Architecture::AArch64
    }

    fn dynamic_tag_values(&self) -> Option<DynamicTagValues> {
        None
    }

    fn symbol_count(&self) -> usize {
        self.symbols.len()
    }
}

pub(crate) trait Platform {
    type ObjectFile<'data>: ObjectFile<'data>;
}

pub(crate) struct ElfPlatform;

impl Platform for ElfPlatform {
    type ObjectFile<'data> = File<'data>;
}
