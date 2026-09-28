use cranpose_plugin_authoring::{Catalog, runtime};

#[test]
fn byte_colors_are_one_live_color_and_keep_constructor_comments_and_integer_style() {
    let source = "fn palette(){ Color::from_rgba_u8(\n0xffu8, /* green */ 0x80,\n0, 255); Color::from_rgb_u8(12,34,56); Color::rgb(0.1,0.2,0.3); }";
    let catalog = Catalog::parse(source).expect("parse colors");
    assert_eq!(catalog.literals.len(), 3);
    assert!(catalog.literals.iter().all(|v| v.kind == "color"));
    assert_eq!(
        runtime::color_channels(&catalog.literals[0].value).expect("RGBA"),
        [1.0, 128.0 / 255.0, 0.0, 1.0]
    );
    let edited = catalog
        .replacement(source, 0, "0,1,0.5,0.25")
        .expect("edit RGBA");
    assert!(edited.contains("0x00u8, /* green */ 0xff,\n128, 64"));
    assert_eq!(edited.lines().count(), source.lines().count());
    assert_eq!(
        Catalog::parse(&edited).expect("edited").schema,
        catalog.schema
    );
    let rgb = catalog
        .replacement(source, 1, "1,0,0.5,1")
        .expect("edit RGB");
    assert!(rgb.contains("Color::from_rgb_u8(255,0,128)"));
    assert!(catalog.replacement(source, 1, "1,0,0,0.5").is_err());
    let instrumented = catalog.instrument(source, "colors.rs");
    syn::parse_file(&instrumented).expect("instrumented colors");
    assert_eq!(instrumented.lines().count(), source.lines().count());
    let excluded = "const C: Color = Color::from_rgba_u8(1,2,3,4); const fn c()->Color{Color::from_rgb_u8(1,2,3)} #[composable] fn App(){remember(||Color::from_rgb_u8(1,2,3));}";
    assert!(
        Catalog::parse(excluded)
            .expect("excluded")
            .literals
            .is_empty()
    );
}

#[test]
fn byte_color_instrumentation_compiles_and_applies_quantized_runtime_channels() {
    let source = "fn main(){let c=Color::from_rgba_u8(10u8,20,30,40); let rgb=Color::from_rgb_u8(10,20,30); let f=Color::rgb(0.1f32,0.2,0.3); println!(\"{:?}|{:?}|{:?}\",c,rgb,f);}";
    let catalog = Catalog::parse(source).expect("catalog");
    let shim = r#"
#[derive(Debug)] struct Color(f32,f32,f32,f32);
impl Color {
fn from_rgba_u8(r:u8,g:u8,b:u8,a:u8)->Self{Self(r as f32/255.0,g as f32/255.0,b as f32/255.0,a as f32/255.0)}
fn from_rgb_u8(r:u8,g:u8,b:u8)->Self{Self::from_rgba_u8(r,g,b,255)}
fn rgb(r:f32,g:f32,b:f32)->Self{Self(r,g,b,1.0)}
}
mod __cranpose_dev {pub fn literal(_: &str,_:&str,_:usize,_:[f32;4])->[f32;4]{[1.0,0.0,128.0/255.0,64.0/255.0]}}
"#;
    let dir = std::env::temp_dir().join(format!(
        "cranpose-byte-colors-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&dir).expect("directory");
    let path = dir.join("colors.rs");
    std::fs::write(
        &path,
        format!("{shim}\n{}", catalog.instrument(source, "colors.rs")),
    )
    .expect("source");
    let binary = dir.join(format!("colors{}", std::env::consts::EXE_SUFFIX));
    let compiled = std::process::Command::new("rustc")
        .arg("--edition=2024")
        .arg(&path)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("rustc");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let output = std::process::Command::new(&binary).output().expect("run");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Color(1.0, 0.0, 0.5019608, 0.2509804)|Color(1.0, 0.0, 0.5019608, 1.0)|Color(1.0, 0.0, 0.5019608, 1.0)"
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn multiline_color_edits_keep_comments_suffixes_and_source_lines() {
    let source = "#[composable] fn App(){ let c=Color::rgba(\n0.1f32, /* red 🙂 */ 0.2,\n 0.3, 1.0);\nText(c); }";
    let catalog = Catalog::parse(source).expect("catalog");
    let edited = catalog
        .replacement(source, 0, "0.9,0.8,0.7,0.5")
        .expect("edit");
    assert!(edited.contains("0.9f32, /* red 🙂 */ 0.8,"));
    assert_eq!(source.lines().count(), edited.lines().count());
    assert_eq!(
        catalog.schema,
        Catalog::parse(&edited).expect("parse").schema
    );
    syn::parse_file(&catalog.instrument(source, "app.rs")).expect("instrument");
}

#[test]
fn palette_colors_are_grouped_and_constants_stay_compiled() {
    let source = "const WHITE: Color = Color(1.0,1.0,1.0,1.0); const fn c()->Color{Color(1.0,1.0,1.0,1.0)} fn palette()->Color{Color(0.1,0.2,0.3,1.0)} #[composable] fn App(){let c=remember(||Color(0.4,0.5,0.6,1.0)); Text(Color(0.2,0.4,0.6,1.0));}";
    let catalog = Catalog::parse(source).expect("parse");
    assert_eq!(catalog.literals.len(), 2);
    assert!(catalog.literals.iter().all(|v| v.kind == "color"));
    let next = catalog
        .replacement(source, 0, "0,0.25,0.8,0.5")
        .expect("color");
    assert_eq!(Catalog::parse(&next).expect("parse").schema, catalog.schema);
    assert!(catalog.replacement(source, 0, "1,2,3,4").is_err());
    assert!(catalog.replacement(source, 0, "NaN,0,0,1").is_err());
    syn::parse_file(&catalog.instrument(source, "palette.rs")).expect("instrumented palette");
    let mut store = runtime::Store::default();
    assert_eq!(store.value("palette.rs", "a", 0, [0.0f32; 4]), [0.0; 4]);
    store
        .apply(&runtime::Update {
            file: "palette.rs".into(),
            schema: "a".into(),
            revision: 1,
            values: vec![runtime::Value {
                id: 0,
                kind: "color".into(),
                value: "0,0.25,0.8,0.5".into(),
            }],
        })
        .expect("apply");
    assert_eq!(
        store.value("palette.rs", "a", 0, [0.0f32; 4]),
        [0.0, 0.25, 0.8, 0.5]
    );
}

#[test]
fn format_text_changes_are_live_but_fields_and_other_macros_are_compiled() {
    let source = r##"#[composable] fn App(){ Text(format!(r#"Hello {{world}} {name}: {0:>width$} /* */"#, 7, width=3, /* trailing */)); let x = format_args!("unchanged {}", 1); }"##;
    let catalog = Catalog::parse(source).expect("parse");
    assert_eq!(catalog.formats.len(), 1);
    let changed = catalog
        .replacement(source, 0, "Goodbye {name} = {0:>width$}!")
        .expect("replace text");
    assert_eq!(
        catalog.schema,
        Catalog::parse(&changed).expect("parse").schema
    );
    assert!(
        catalog
            .replacement(source, 0, "Changed {name:?} = {0:>width$}!")
            .is_err()
    );
    assert!(catalog.replacement(source, 0, "Unmatched {").is_err());
    syn::parse_file(&catalog.instrument(source, "main.rs")).expect("instrumented macro");
    assert_eq!(
        runtime::format_text("Hello {{world}} {name} {}!", 0),
        "Hello {world} "
    );
    assert_eq!(runtime::format_text("Hello {{world}} {name} {}!", 2), "!");
}

#[test]
fn instrumented_format_preserves_implicit_captures_positions_specs_and_evaluation() {
    let body = r##"fn main(){let name="Cranpose"; let width=6; let n=3; let mut count=0; let text=format!(r#"{{start}} {name} {1:>width$} {0:.1} {} {:?} /* */"#, {count+=1; 1.25}, n,); println!("{text}|{count}");}"##;
    let source = body.replacen("fn main", "#[composable] fn main", 1);
    let catalog = Catalog::parse(&source).expect("parse");
    let text = &catalog.literals[catalog.formats[0].literal].value;
    let (parts, _) = runtime::format_parts(text).expect("format");
    let instrumented = catalog
        .instrument(&source, "main.rs")
        .replace("#[composable]", "");
    let shim = format!(
        "mod __cranpose_dev {{ pub fn literal<T>(_:&str,_:&str,_:usize,v:T)->T{{v}} pub mod values {{pub fn format_text(_:&str,index:usize)->String{{ {:?}[index].to_string() }} }} }}",
        parts
    );
    let dir = std::env::temp_dir().join(format!(
        "cranpose-format-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&dir).expect("directory");
    let mut outputs = Vec::new();
    for (name, source) in [
        ("baseline", body.to_owned()),
        ("candidate", format!("{shim}\n{instrumented}")),
    ] {
        let path = dir.join(format!("{name}.rs"));
        std::fs::write(&path, source).expect("source");
        let binary = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        let compiled = std::process::Command::new("rustc")
            .arg("--edition=2024")
            .arg(&path)
            .arg("-o")
            .arg(&binary)
            .output()
            .expect("rustc");
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let output = std::process::Command::new(&binary).output().expect("run");
        assert!(output.status.success());
        outputs.push(output.stdout);
    }
    assert_eq!(outputs[0], outputs[1]);
    std::fs::remove_dir_all(dir).expect("cleanup");
}
