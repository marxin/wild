use crate::elf::File;
use crate::macho::MachOFile;

pub(crate) trait ObjectFile<'data>: Send {}

impl<'data> ObjectFile<'data> for File<'data> {}

impl<'data> ObjectFile<'data> for MachOFile<'data> {}

pub(crate) trait Platform {
    type ObjectFile<'data>: ObjectFile<'data>;
}

pub(crate) struct ElfPlatform;

impl Platform for ElfPlatform {
    type ObjectFile<'data> = File<'data>;
}
