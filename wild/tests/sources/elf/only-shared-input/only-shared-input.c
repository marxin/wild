//#Config:default
//#Mode:dynamic
//#RunEnabled:false
//#Shared:only-shared-input-lib.c
//#LinkArgs:-shared -z now

//#DiffIgnore:file-header.entry
//#DiffIgnore:.dynamic.DT_RELA
//#DiffIgnore:.dynamic.DT_RELAENT
//#DiffIgnore:.dynamic.DT_NEEDED

// This file is intentionally not passed to the final link. The test links a shared object whose
// only input is the DSO built from only-shared-input-lib.c.
