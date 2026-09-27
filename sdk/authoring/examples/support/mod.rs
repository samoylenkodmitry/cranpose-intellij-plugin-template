use std::fmt::Write;

pub const CASES: [(usize, bool, bool); 6] = [
    (32, false, false),
    (256, false, false),
    (1024, false, false),
    (2048, false, false),
    (1024, true, false),
    (32, true, true),
];

pub fn fixture(functions: usize, unicode: bool, single_line: bool) -> String {
    let label = if unicode {
        "Café 🦀 observation"
    } else {
        "Field observation"
    };
    let separator = if single_line { " " } else { "\n" };
    let mut source = String::new();
    for i in 0..functions {
        writeln!(source, "#[composable] fn Card{i}() {{").expect("string");
        for _ in 0..if single_line { 64 } else { 1 } {
            write!(source, "    Column(Modifier::empty(), || {{ Text({label:?}, Modifier::empty().padding(14.0)); }});{separator}").expect("string");
        }
        writeln!(source, "}}").expect("string");
    }
    source
}
