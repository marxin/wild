//#LinkerDriver:clang
//#LinkArgs:-Wl,-rpath,/wild-rpath-test/a -Wl,-rpath,/wild-rpath-test/b -Wl,-rpath,/wild-rpath-test/a -Wl,-rpath,@loader_path/wild-rpath-test
//#Contains:/wild-rpath-test/a
//#Contains:/wild-rpath-test/b
//#Contains:@loader_path/wild-rpath-test

int main() { return 42; }
