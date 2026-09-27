use cranpose_plugin_authoring::Catalog;

#[test]
fn variable_controls_follow_initializer_and_aliases_without_replacing_uses() {
    let source = "#[composable] fn App(){ let spacing: f32 = 24.0; let gap = spacing; let title = \"Hello\"; Column(||{Text(title); padding(gap);}); }";
    let catalog = Catalog::parse(source).expect("catalog");
    let names: Vec<_> = catalog
        .references
        .iter()
        .map(|r| (r.name.as_str(), r.literal))
        .collect();
    assert_eq!(
        names,
        [
            ("spacing", 0),
            ("gap", 0),
            ("spacing", 0),
            ("title", 1),
            ("title", 1),
            ("gap", 0)
        ]
    );
    let changed = catalog
        .replacement(
            source,
            catalog.references.last().expect("gap").literal,
            "32.0",
        )
        .expect("edit initializer");
    assert!(changed.contains("spacing: f32 = 32.0"));
    assert!(changed.contains("padding(gap)"));
    assert_eq!(
        catalog.schema,
        Catalog::parse(&changed).expect("changed").schema
    );
}

#[test]
fn color_fields_tuple_bindings_and_unicode_references_resolve() {
    let source = "// 🦀\n#[composable] fn App(){let café=Color(0.19,0.42,0.31,1.0); let palette=Ink{accent:café}; paint(palette.accent); let Ink{accent:ink}=palette; paint(ink); let (a,b)=(4,8); size(b,a);}";
    let catalog = Catalog::parse(source).expect("catalog");
    for r in catalog
        .references
        .iter()
        .filter(|r| ["café", "palette.accent", "ink"].contains(&r.name.as_str()))
    {
        assert_eq!(catalog.literals[r.literal].kind, "color");
        assert_eq!(
            r.range.start_utf16,
            source[..r.range.start].encode_utf16().count()
        );
        assert_eq!(&source[r.range.start..r.range.end], r.name);
    }
    assert!(
        catalog
            .references
            .iter()
            .any(|r| r.name == "palette.accent")
    );
    assert!(
        catalog
            .references
            .iter()
            .any(|r| r.name == "b" && catalog.literals[r.literal].value == "8")
    );
}

#[test]
fn shadowing_and_compiler_owned_contexts_never_tune_an_outer_binding() {
    let source = "#[composable] fn App(){let n=12; use_n(n); { let n = unknown(); use_n(n); } let f=|n|use_n(n); match value { Some(n) => use_n(n), _ => () }; for n in items {use_n(n);} if let Some(n)=value {use_n(n);} else {use_n(n);} while let Some(n)=value {use_n(n);} remember(||use_n(n)); key(n,||use_n(n)); let a=[0;n]; let b=&n; const {use_n(n);}; {const n:i32=7;use_n(n);} fn nested(){use_n(n);} let mut n=4; use_n(n); }";
    let catalog = Catalog::parse(source).expect("catalog");
    let refs: Vec<_> = catalog
        .references
        .iter()
        .filter(|r| r.name == "n")
        .collect();
    // Declaration, normal use, the else branch and the key's content only.
    assert_eq!(refs.len(), 4);
    assert!(
        refs.iter()
            .all(|r| catalog.literals[r.literal].value == "12")
    );
}

#[test]
fn imports_rest_patterns_and_ambiguous_expressions_are_conservative() {
    let source = "#[composable] fn App(){let n=12; {use other::n; use_n(n);} let (first,..,last)=(1,2,3,4); use_n(last); let calculated=n+1; use_n(calculated); let remembered=remember(||3); use_n(remembered); let n=n; use_n(n);}";
    let catalog = Catalog::parse(source).expect("catalog");
    assert!(
        !catalog
            .references
            .iter()
            .any(|r| ["last", "first", "calculated", "remembered"].contains(&r.name.as_str()))
    );
    assert_eq!(
        catalog.references.iter().filter(|r| r.name == "n").count(),
        5
    );
}
