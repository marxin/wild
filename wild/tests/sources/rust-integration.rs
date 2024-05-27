//#AbstractConfig:default
//#DiffIgnore:asm.dummy
//#LinkArgs:static:--cc=clang -static -C relocation-model=static

//#Config:llvm:default
//#CompArgs:--target x86_64-unknown-linux-musl -C relocation-model=static -C target-feature=+crt-static -C debuginfo=2

//#Config:cranelift:default
//#CompArgs:-Zcodegen-backend=cranelift --target x86_64-unknown-linux-musl -C relocation-model=static -C target-feature=+crt-static -C debuginfo=2

fn foo() {
    panic!("Make sure unwinding works");
}

fn main() {
    if std::panic::catch_unwind(foo).is_ok() {
        std::process::exit(101);
    }
    std::process::exit(42);
}
