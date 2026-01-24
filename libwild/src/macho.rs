use object::{
    Endianness,
    macho::{self, Section64, SegmentCommand64, SymtabCommand},
    read::macho::{MachHeader, Nlist, Section, Segment},
};

use crate::{ensure, error::Result};

const LE: Endianness = Endianness::Little;

type SectionTable<'data> = &'data [Section64<crate::macho::Endianness>];
type SymbolTable<'data> = object::read::macho::SymbolTable<'data, macho::MachHeader64<Endianness>>;

#[derive(derive_more::Debug)]
pub(crate) struct MachOFile<'data> {
    #[debug(skip)]
    pub(crate) data: &'data [u8],
    #[debug(skip)]
    pub(crate) sections: SectionTable<'data>,
    #[debug(skip)]
    pub(crate) symbols: SymbolTable<'data>,
    pub(crate) flags: u32,
}

impl<'data> MachOFile<'data> {
    pub(crate) fn parse(data: &'data [u8]) -> Result<Self> {
        let header = macho::MachHeader64::<object::Endianness>::parse(data, 0)?;
        let mut commands = header.load_commands(LE, &*data, 0)?;

        let mut symbols = None;
        let mut sections = None;

        while let Some(command) = commands.next()? {
            if let Some(symtab_command) = command.symtab()? {
                ensure!(symbols.is_none(), "At most one symtab command expected");
                symbols = Some(symtab_command.symbols::<macho::MachHeader64<_>, _>(LE, data)?);
            } else if let Some((segment_command, segment_data)) = command.segment_64()? {
                ensure!(sections.is_none(), "At most one segment command expected");
                sections = Some(segment_command.sections(LE, segment_data)?);
            }
        }

        Ok(MachOFile {
            data,
            symbols: symbols.ok_or("Missing symbol table")?,
            sections: sections.ok_or("Missing segment command")?,
            flags: header.flags(LE),
        })
    }
}
