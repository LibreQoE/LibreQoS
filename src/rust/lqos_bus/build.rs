fn main() {
    cc::Build::new()
        .file("src/tc_handle_parser.c")
        .compile("tc_handle_parse.o");
}