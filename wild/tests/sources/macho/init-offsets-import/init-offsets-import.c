//#LinkerDriver:clang
//#Shared:initializer.c
//#ExpectErrorWild:Mach-O initializer is defined externally

void imported_initializer(void);

__attribute__((
    used,
    section("__DATA,__mod_init_func,mod_init_funcs"))) static void (*const initializers[])(void) = {
    imported_initializer};

int main(void) { return 42; }
