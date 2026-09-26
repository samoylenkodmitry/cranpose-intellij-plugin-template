//! Minimal JVM class-file generation for native Rust IDE extensions.
//!
//! Generated methods load the native library, call superclass constructors,
//! box arguments and forward calls. All application behavior lives in Rust.
//! Reference: JVMS 4 (class files) and 6 (instructions).
pub mod intellij;
use std::collections::BTreeMap;

pub const BRIDGE: &str = "dev/cranpose/rust/Native";
pub const DISPATCH: &str =
    "(Ljava/lang/String;Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;";

#[derive(Default)]
struct Pool {
    entries: Vec<Vec<u8>>,
    indexes: BTreeMap<Vec<u8>, u16>,
}
impl Pool {
    fn entry(&mut self, value: Vec<u8>) -> u16 {
        if let Some(index) = self.indexes.get(&value) {
            return *index;
        }
        let index = u16::try_from(self.entries.len() + 1).expect("constant pool overflow");
        self.entries.push(value.clone());
        self.indexes.insert(value, index);
        index
    }
    fn utf8(&mut self, text: &str) -> u16 {
        let mut encoded = Vec::new();
        for unit in text.encode_utf16() {
            match unit {
                1..=0x7f => encoded.push(unit as u8),
                0..=0x7ff => {
                    encoded.push(0xc0 | (unit >> 6) as u8);
                    encoded.push(0x80 | (unit & 63) as u8);
                }
                _ => {
                    encoded.push(0xe0 | (unit >> 12) as u8);
                    encoded.push(0x80 | ((unit >> 6) & 63) as u8);
                    encoded.push(0x80 | (unit & 63) as u8);
                }
            }
        }
        let mut value = vec![1];
        put16(
            &mut value,
            u16::try_from(encoded.len()).expect("UTF8 constant too long"),
        );
        value.extend(encoded);
        self.entry(value)
    }
    fn class(&mut self, name: &str) -> u16 {
        let name = self.utf8(name);
        self.pair(7, name)
    }
    fn string(&mut self, value: &str) -> u16 {
        let value = self.utf8(value);
        self.pair(8, value)
    }
    fn pair(&mut self, tag: u8, value: u16) -> u16 {
        let mut bytes = vec![tag];
        put16(&mut bytes, value);
        self.entry(bytes)
    }
    fn member(&mut self, tag: u8, class: &str, name: &str, signature: &str) -> u16 {
        let class = self.class(class);
        let name = self.utf8(name);
        let signature = self.utf8(signature);
        let mut nt = vec![12];
        put16(&mut nt, name);
        put16(&mut nt, signature);
        let nt = self.entry(nt);
        let mut bytes = vec![tag];
        put16(&mut bytes, class);
        put16(&mut bytes, nt);
        self.entry(bytes)
    }
}
fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend(value.to_be_bytes());
}
fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_be_bytes());
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Void,
    Boolean,
    Byte,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
    Object(String),
}
impl Type {
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, String> {
        let start = *pos;
        let code = *bytes.get(*pos).ok_or("truncated descriptor")?;
        *pos += 1;
        Ok(match code {
            b'V' => Self::Void,
            b'Z' => Self::Boolean,
            b'B' => Self::Byte,
            b'C' => Self::Char,
            b'S' => Self::Short,
            b'I' => Self::Int,
            b'J' => Self::Long,
            b'F' => Self::Float,
            b'D' => Self::Double,
            b'L' => {
                let name_start = *pos;
                while bytes.get(*pos).is_some_and(|c| *c != b';') {
                    *pos += 1;
                }
                if *pos == name_start || bytes.get(*pos) != Some(&b';') {
                    return Err("invalid object descriptor".into());
                }
                let name = std::str::from_utf8(&bytes[name_start..*pos])
                    .map_err(|e| e.to_string())?
                    .to_owned();
                *pos += 1;
                Self::Object(name)
            }
            b'[' => {
                if Self::read(bytes, pos)? == Self::Void {
                    return Err("void array".into());
                }
                Self::Object(
                    std::str::from_utf8(&bytes[start..*pos])
                        .map_err(|e| e.to_string())?
                        .to_owned(),
                )
            }
            _ => return Err("invalid descriptor".into()),
        })
    }
    fn slots(&self) -> u16 {
        match self {
            Self::Void => 0,
            Self::Long | Self::Double => 2,
            _ => 1,
        }
    }
    fn descriptor(&self) -> String {
        match self {
            Self::Void => "V".into(),
            Self::Boolean => "Z".into(),
            Self::Byte => "B".into(),
            Self::Char => "C".into(),
            Self::Short => "S".into(),
            Self::Int => "I".into(),
            Self::Long => "J".into(),
            Self::Float => "F".into(),
            Self::Double => "D".into(),
            Self::Object(name) if name.starts_with('[') => name.clone(),
            Self::Object(name) => format!("L{name};"),
        }
    }
    fn wrapper(&self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Self::Boolean => ("java/lang/Boolean", "booleanValue"),
            Self::Byte => ("java/lang/Byte", "byteValue"),
            Self::Char => ("java/lang/Character", "charValue"),
            Self::Short => ("java/lang/Short", "shortValue"),
            Self::Int => ("java/lang/Integer", "intValue"),
            Self::Long => ("java/lang/Long", "longValue"),
            Self::Float => ("java/lang/Float", "floatValue"),
            Self::Double => ("java/lang/Double", "doubleValue"),
            _ => return None,
        })
    }
}
pub fn descriptor(signature: &str) -> Result<(Vec<Type>, Type), String> {
    let bytes = signature.as_bytes();
    if bytes.first() != Some(&b'(') {
        return Err("expected method descriptor".into());
    }
    let mut pos = 1;
    let mut args = Vec::new();
    while bytes.get(pos) != Some(&b')') {
        let arg = Type::read(bytes, &mut pos)?;
        if arg == Type::Void {
            return Err("void argument".into());
        }
        args.push(arg);
    }
    pos += 1;
    let result = Type::read(bytes, &mut pos)?;
    if pos != bytes.len() {
        return Err("trailing descriptor bytes".into());
    }
    if args.iter().map(Type::slots).sum::<u16>() > 254 {
        return Err("too many argument slots".into());
    }
    Ok((args, result))
}

/// A straight-line bytecode body, with no branches or exception handlers.
pub struct Code<'a> {
    pool: &'a mut Pool,
    bytes: Vec<u8>,
}
impl Code<'_> {
    pub fn op(&mut self, op: u8) {
        self.bytes.push(op);
    }
    pub fn indexed(&mut self, op: u8, index: u16) {
        self.op(op);
        put16(&mut self.bytes, index);
    }
    pub fn text(&mut self, text: &str) {
        let index = self.pool.string(text);
        self.indexed(0x13, index);
    }
    pub fn class(&mut self, class: &str) {
        let index = self.pool.class(class);
        self.indexed(0x13, index);
    }
    pub fn integer(&mut self, value: i16) {
        self.op(0x11);
        self.bytes.extend(value.to_be_bytes());
    }
    pub fn null(&mut self) {
        self.op(0x01);
    }
    pub fn load(&mut self, kind: &Type, local: u16) {
        let op = match kind {
            Type::Long => 0x16,
            Type::Float => 0x17,
            Type::Double => 0x18,
            Type::Object(_) => 0x19,
            _ => 0x15,
        };
        if local > 255 {
            self.op(0xc4);
            self.op(op);
            put16(&mut self.bytes, local);
        } else {
            self.op(op);
            self.op(local as u8);
        }
    }
    pub fn object(&mut self, local: u16) {
        self.load(&Type::Object(String::new()), local);
    }
    pub fn new_object(&mut self, class: &str) {
        let index = self.pool.class(class);
        self.indexed(0xbb, index);
        self.op(0x59);
    }
    pub fn field(&mut self, op: u8, class: &str, name: &str, signature: &str) {
        let index = self.pool.member(9, class, name, signature);
        self.indexed(op, index);
    }
    pub fn invoke(&mut self, op: u8, class: &str, name: &str, signature: &str) {
        let index = self
            .pool
            .member(if op == 0xb9 { 11 } else { 10 }, class, name, signature);
        self.indexed(op, index);
        if op == 0xb9 {
            let (args, _) = descriptor(signature).expect("interface descriptor");
            self.op((args.iter().map(Type::slots).sum::<u16>() + 1) as u8);
            self.op(0);
        }
    }
    pub fn cast(&mut self, class: &str) {
        let index = self.pool.class(class);
        self.indexed(0xc0, index);
    }
    fn argument_array(&mut self, args: &[Type], first: u16) {
        self.integer(i16::try_from(args.len()).expect("argument count"));
        let object = self.pool.class("java/lang/Object");
        self.indexed(0xbd, object);
        let mut local = first;
        for (index, kind) in args.iter().enumerate() {
            self.op(0x59);
            self.integer(index as i16);
            self.load(kind, local);
            if let Some((wrapper, _)) = kind.wrapper() {
                self.invoke(
                    0xb8,
                    wrapper,
                    "valueOf",
                    &format!("({})L{wrapper};", kind.descriptor()),
                );
            }
            self.op(0x53);
            local += kind.slots();
        }
    }
    fn dispatch(&mut self, operation: &str, args: &[Type], is_static: bool) {
        self.text(operation);
        if is_static {
            self.null();
        } else {
            self.object(0);
        }
        self.argument_array(args, if is_static { 0 } else { 1 });
        self.invoke(0xb8, BRIDGE, "call", DISPATCH);
    }
    fn return_value(&mut self, kind: &Type) {
        if let Some((wrapper, accessor)) = kind.wrapper() {
            self.cast(wrapper);
            self.invoke(0xb6, wrapper, accessor, &format!("(){}", kind.descriptor()));
            self.op(match kind {
                Type::Long => 0xad,
                Type::Float => 0xae,
                Type::Double => 0xaf,
                _ => 0xac,
            });
        } else if let Type::Object(class) = kind {
            self.cast(class);
            self.op(0xb0);
        } else {
            self.op(0x57);
            self.op(0xb1);
        }
    }
}

struct Member {
    access: u16,
    name: u16,
    signature: u16,
    code: Option<Vec<u8>>,
    locals: u16,
}
pub struct Class {
    pool: Pool,
    name: String,
    parent: String,
    interfaces: Vec<u16>,
    fields: Vec<Member>,
    methods: Vec<Member>,
}
impl Class {
    pub fn new(name: &str, parent: &str, interfaces: &[&str]) -> Self {
        let mut pool = Pool::default();
        let interfaces = interfaces.iter().map(|v| pool.class(v)).collect();
        Self {
            pool,
            name: name.into(),
            parent: parent.into(),
            interfaces,
            fields: vec![],
            methods: vec![],
        }
    }
    pub fn field(&mut self, name: &str, signature: &str) {
        self.fields.push(Member {
            access: 0x0001,
            name: self.pool.utf8(name),
            signature: self.pool.utf8(signature),
            code: None,
            locals: 0,
        });
    }
    pub fn method(
        &mut self,
        access: u16,
        name: &str,
        signature: &str,
        body: impl FnOnce(&mut Code<'_>),
    ) {
        let (args, _) = descriptor(signature).expect("valid method descriptor");
        let locals = args.iter().map(Type::slots).sum::<u16>() + u16::from(access & 8 == 0);
        let mut code = Code {
            pool: &mut self.pool,
            bytes: Vec::new(),
        };
        body(&mut code);
        let bytes = code.bytes;
        self.methods.push(Member {
            access,
            name: self.pool.utf8(name),
            signature: self.pool.utf8(signature),
            code: Some(bytes),
            locals,
        });
    }
    pub fn native(&mut self, name: &str, signature: &str) {
        descriptor(signature).expect("native descriptor");
        self.methods.push(Member {
            access: 0x0109,
            name: self.pool.utf8(name),
            signature: self.pool.utf8(signature),
            code: None,
            locals: 0,
        });
    }
    pub fn forward(&mut self, name: &str, signature: &str) {
        let operation = format!(
            "{}.{}",
            self.name.rsplit('/').next().expect("class name"),
            name
        );
        let (args, result) = descriptor(signature).expect("forward descriptor");
        self.method(1, name, signature, |c| {
            c.dispatch(&operation, &args, false);
            c.return_value(&result);
        });
    }
    /// Call the superclass constructor, then optionally notify the Rust host.
    /// The indexes in super_args select zero-based arguments from this constructor.
    pub fn constructor(
        &mut self,
        signature: &str,
        super_signature: &str,
        super_args: &[usize],
        notify: bool,
    ) {
        let parent = self.parent.clone();
        let (args, _) = descriptor(signature).expect("constructor descriptor");
        let operation = format!(
            "{}.<init>",
            self.name.rsplit('/').next().expect("class name")
        );
        self.method(1, "<init>", signature, |c| {
            c.object(0);
            for index in super_args {
                let local = 1 + args[..*index].iter().map(Type::slots).sum::<u16>();
                c.load(&args[*index], local);
            }
            c.invoke(0xb7, &parent, "<init>", super_signature);
            if notify {
                c.dispatch(&operation, &args, false);
                c.op(0x57);
            }
            c.op(0xb1);
        });
    }
    pub fn forward_after_super(&mut self, name: &str, signature: &str) {
        let parent = self.parent.clone();
        let operation = format!(
            "{}.{}",
            self.name.rsplit('/').next().expect("class name"),
            name
        );
        let (args, result) = descriptor(signature).expect("method descriptor");
        assert_eq!(result, Type::Void, "super-prefix methods return void");
        self.method(1, name, signature, |c| {
            c.object(0);
            let mut local = 1;
            for kind in &args {
                c.load(kind, local);
                local += kind.slots();
            }
            c.invoke(0xb7, &parent, name, signature);
            c.dispatch(&operation, &args, false);
            c.return_value(&result);
        });
    }
    pub fn handle_constructor(&mut self) {
        let name = self.name.clone();
        let parent = self.parent.clone();
        self.field("nativeId", "J");
        self.method(1, "<init>", "(J)V", |c| {
            c.object(0);
            c.invoke(0xb7, &parent, "<init>", "()V");
            c.object(0);
            c.load(&Type::Long, 1);
            c.field(0xb5, &name, "nativeId", "J");
            c.op(0xb1);
        });
    }
    pub fn finish(mut self) -> Vec<u8> {
        let this = self.pool.class(&self.name);
        let parent = self.pool.class(&self.parent);
        let code_name = self.pool.utf8("Code");
        let mut out = vec![0xca, 0xfe, 0xba, 0xbe];
        put16(&mut out, 0);
        put16(&mut out, 52);
        put16(&mut out, (self.pool.entries.len() + 1) as u16);
        for value in self.pool.entries {
            out.extend(value);
        }
        put16(&mut out, 0x0021);
        put16(&mut out, this);
        put16(&mut out, parent);
        put16(&mut out, self.interfaces.len() as u16);
        for interface in self.interfaces {
            put16(&mut out, interface);
        }
        for members in [self.fields, self.methods] {
            put16(&mut out, members.len() as u16);
            for member in members {
                put16(&mut out, member.access);
                put16(&mut out, member.name);
                put16(&mut out, member.signature);
                put16(&mut out, u16::from(member.code.is_some()));
                if let Some(code) = member.code {
                    put16(&mut out, code_name);
                    put32(&mut out, code.len() as u32 + 12);
                    put16(&mut out, 512);
                    put16(&mut out, member.locals);
                    put32(&mut out, code.len() as u32);
                    out.extend(code);
                    put16(&mut out, 0);
                    put16(&mut out, 0);
                }
            }
        }
        put16(&mut out, 0);
        out
    }
}

pub fn native_bridge(plugin_id: Option<&str>) -> Vec<u8> {
    let mut class = Class::new(BRIDGE, "java/lang/Object", &[]);
    class.native("call", DISPATCH);
    class.method(0x0008, "<clinit>", "()V", |c| {
        // Load <plugin>/lib/native/<JVM os.arch>/<mapped-library-name>.
        c.new_object("java/io/File");
        c.new_object("java/io/File");
        c.new_object("java/io/File");
        c.new_object("java/io/File");
        if let Some(plugin_id) = plugin_id {
            c.text(plugin_id);
            c.invoke(0xb8, "com/intellij/openapi/extensions/PluginId", "getId", "(Ljava/lang/String;)Lcom/intellij/openapi/extensions/PluginId;");
            c.invoke(0xb8, "com/intellij/ide/plugins/PluginManagerCore", "getPlugin", "(Lcom/intellij/openapi/extensions/PluginId;)Lcom/intellij/ide/plugins/IdeaPluginDescriptor;");
            c.invoke(0xb9, "com/intellij/ide/plugins/IdeaPluginDescriptor", "getPluginPath", "()Ljava/nio/file/Path;");
            c.invoke(0xb9, "java/nio/file/Path", "toFile", "()Ljava/io/File;");
            c.text("lib");
            c.invoke(0xb7, "java/io/File", "<init>", "(Ljava/io/File;Ljava/lang/String;)V");
        } else {
        c.class(BRIDGE);
        c.invoke(
            0xb6,
            "java/lang/Class",
            "getProtectionDomain",
            "()Ljava/security/ProtectionDomain;",
        );
        c.invoke(
            0xb6,
            "java/security/ProtectionDomain",
            "getCodeSource",
            "()Ljava/security/CodeSource;",
        );
        c.invoke(
            0xb6,
            "java/security/CodeSource",
            "getLocation",
            "()Ljava/net/URL;",
        );
        c.invoke(0xb6, "java/net/URL", "toURI", "()Ljava/net/URI;");
        c.invoke(0xb7, "java/io/File", "<init>", "(Ljava/net/URI;)V");
        c.invoke(0xb6, "java/io/File", "getParentFile", "()Ljava/io/File;");
        }
        c.text("native");
        c.invoke(
            0xb7,
            "java/io/File",
            "<init>",
            "(Ljava/io/File;Ljava/lang/String;)V",
        );
        c.text("os.arch");
        c.invoke(
            0xb8,
            "java/lang/System",
            "getProperty",
            "(Ljava/lang/String;)Ljava/lang/String;",
        );
        c.invoke(
            0xb7,
            "java/io/File",
            "<init>",
            "(Ljava/io/File;Ljava/lang/String;)V",
        );
        c.text("cranpose_ide_host");
        c.invoke(
            0xb8,
            "java/lang/System",
            "mapLibraryName",
            "(Ljava/lang/String;)Ljava/lang/String;",
        );
        c.invoke(
            0xb7,
            "java/io/File",
            "<init>",
            "(Ljava/io/File;Ljava/lang/String;)V",
        );
        c.invoke(
            0xb6,
            "java/io/File",
            "getAbsolutePath",
            "()Ljava/lang/String;",
        );
        c.invoke(0xb8, "java/lang/System", "load", "(Ljava/lang/String;)V");
        c.op(0xb1);
    });
    class.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptors_cover_wide_values_and_arrays() {
        assert_eq!(
            descriptor("(JDLjava/lang/String;[[I)Z").expect("descriptor"),
            (
                vec![
                    Type::Long,
                    Type::Double,
                    Type::Object("java/lang/String".into()),
                    Type::Object("[[I".into())
                ],
                Type::Boolean
            )
        );
    }
    #[test]
    fn malformed_descriptors_are_rejected() {
        for invalid in [
            "",
            "I",
            "(V)V",
            "([V)V",
            "(L;)V",
            "(I",
            "()Vx",
            "(Ljava/lang/String)V",
        ] {
            assert!(descriptor(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn constants_use_modified_utf8() {
        let mut pool = Pool::default();
        pool.utf8("\0🦀");
        assert_eq!(
            pool.entries[0],
            [1, 0, 8, 0xc0, 0x80, 0xed, 0xa0, 0xbe, 0xed, 0xb6, 0x80]
        );
    }
}
