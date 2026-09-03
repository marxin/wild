//#LinkerDriver:clang
//#ExpectSection:__init_offsets
//#NoSection:__mod_init_func

static int state;

__attribute__((constructor)) static void first(void) { state = 1; }
__attribute__((constructor)) static void second(void) { state = state * 10 + 2; }

int main(void) { return state == 12 ? 42 : 1; }
