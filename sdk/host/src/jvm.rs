//! Checked JNI calls and scoped native callbacks.
use anyhow::{Context, Result};
use jni::{
    JNIEnv,
    objects::{GlobalRef, JObject, JObjectArray, JString, JValue},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicI64, Ordering},
    },
};

pub type O = GlobalRef;
pub enum A<'a> {
    O(&'a O),
    S(&'a str),
    I(i32),
    J(i64),
    F(f32),
    D(f64),
    Z(bool),
    Null,
}
pub struct J<'a> {
    pub env: JNIEnv<'a>,
}
impl<'a> J<'a> {
    pub fn global(&self, value: impl AsRef<JObject<'a>>) -> Result<O> {
        Ok(self.env.new_global_ref(value)?)
    }
    fn owned(&mut self, value: JObject<'a>) -> Result<O> {
        let result = self.env.new_global_ref(&value)?;
        self.env.delete_local_ref(value)?;
        Ok(result)
    }
    pub fn null(&self) -> Result<O> {
        self.global(JObject::null())
    }
    pub fn string(&mut self, value: &str) -> Result<O> {
        let s = self.env.new_string(value)?;
        self.owned(s.into())
    }
    pub fn read_string(&mut self, value: &O) -> Result<String> {
        if value.is_null() {
            return Ok(String::new());
        }
        let local = self.env.new_local_ref(value)?;
        let string = JString::from(local);
        let text = self.env.get_string(&string)?.into();
        self.env.delete_local_ref(string)?;
        Ok(text)
    }
    fn objects(&mut self, args: &[A<'_>]) -> Result<Vec<jni::objects::AutoLocal<'a, JObject<'a>>>> {
        args.iter()
            .map(|a| {
                let object = match a {
                    A::S(text) => self.env.new_string(text)?.into(),
                    _ => JObject::null(),
                };
                Ok(self.env.auto_local(object))
            })
            .collect()
    }
    fn values<'b>(
        args: &'b [A<'b>],
        objects: &'b [jni::objects::AutoLocal<'a, JObject<'a>>],
    ) -> Vec<JValue<'b, 'b>>
    where
        'a: 'b,
    {
        args.iter()
            .zip(objects)
            .map(|(a, s)| match a {
                A::O(o) => JValue::Object(o.as_obj()),
                A::S(_) => JValue::Object(s),
                A::I(v) => JValue::Int(*v),
                A::J(v) => JValue::Long(*v),
                A::F(v) => JValue::Float(*v),
                A::D(v) => JValue::Double(*v),
                A::Z(v) => JValue::Bool(u8::from(*v)),
                A::Null => JValue::Object(s),
            })
            .collect()
    }
    #[allow(clippy::wrong_self_convention, clippy::new_ret_no_self)] // Constructs a JVM object, not the JNI context.
    pub fn new(&mut self, class: &str, signature: &str, args: &[A<'_>]) -> Result<O> {
        let objects = self.objects(args)?;
        let values = Self::values(args, &objects);
        let result = self
            .env
            .new_object(class, signature, &values)
            .with_context(|| format!("construct {class}{signature}"))?;
        self.owned(result)
    }
    pub fn call(
        &mut self,
        object: &O,
        name: &str,
        signature: &str,
        args: &[A<'_>],
    ) -> Result<jni::objects::JValueOwned<'a>> {
        let objects = self.objects(args)?;
        let values = Self::values(args, &objects);
        self.env
            .call_method(object, name, signature, &values)
            .with_context(|| format!("call {name}{signature}"))
    }
    pub fn static_call(
        &mut self,
        class: &str,
        name: &str,
        signature: &str,
        args: &[A<'_>],
    ) -> Result<jni::objects::JValueOwned<'a>> {
        let objects = self.objects(args)?;
        let values = Self::values(args, &objects);
        self.env
            .call_static_method(class, name, signature, &values)
            .with_context(|| format!("call {class}.{name}{signature}"))
    }
    pub fn obj(&mut self, object: &O, name: &str, signature: &str, args: &[A<'_>]) -> Result<O> {
        let result = self.call(object, name, signature, args)?.l()?;
        self.owned(result)
    }
    pub fn static_obj(
        &mut self,
        class: &str,
        name: &str,
        signature: &str,
        args: &[A<'_>],
    ) -> Result<O> {
        let result = self.static_call(class, name, signature, args)?.l()?;
        self.owned(result)
    }
    pub fn void(&mut self, object: &O, name: &str, signature: &str, args: &[A<'_>]) -> Result<()> {
        self.call(object, name, signature, args)?.v()?;
        Ok(())
    }
    pub fn static_void(
        &mut self,
        class: &str,
        name: &str,
        signature: &str,
        args: &[A<'_>],
    ) -> Result<()> {
        self.static_call(class, name, signature, args)?.v()?;
        Ok(())
    }
    pub fn int(&mut self, object: &O, name: &str) -> Result<i32> {
        Ok(self.call(object, name, "()I", &[])?.i()?)
    }
    pub fn long(&mut self, object: &O, name: &str) -> Result<i64> {
        Ok(self.call(object, name, "()J", &[])?.j()?)
    }
    pub fn double(&mut self, object: &O, name: &str) -> Result<f64> {
        Ok(self.call(object, name, "()D", &[])?.d()?)
    }
    pub fn bool(&mut self, object: &O, name: &str) -> Result<bool> {
        Ok(self.call(object, name, "()Z", &[])?.z()?)
    }
    pub fn text(&mut self, object: &O, name: &str) -> Result<String> {
        let result = self.obj(object, name, "()Ljava/lang/String;", &[])?;
        self.read_string(&result)
    }
    pub fn field_int(&mut self, object: &O, name: &str) -> Result<i32> {
        Ok(self.env.get_field(object, name, "I")?.i()?)
    }
    pub fn field_object(&mut self, object: &O, name: &str, signature: &str) -> Result<O> {
        let result = self.env.get_field(object, name, signature)?.l()?;
        self.owned(result)
    }
    pub fn constant(&mut self, class: &str, name: &str, signature: &str) -> Result<O> {
        let result = self.env.get_static_field(class, name, signature)?.l()?;
        self.owned(result)
    }
    pub fn class(&mut self, name: &str) -> Result<O> {
        let result = self.env.find_class(name)?;
        self.owned(result.into())
    }
    pub fn array(&mut self, class: &str, values: &[O]) -> Result<O> {
        let result = self
            .env
            .new_object_array(values.len() as i32, class, JObject::null())?;
        for (index, value) in values.iter().enumerate() {
            self.env
                .set_object_array_element(&result, index as i32, value)?;
        }
        self.owned(result.into())
    }
    pub fn elements(&mut self, array: &O) -> Result<Vec<O>> {
        if array.is_null() {
            return Ok(vec![]);
        }
        let array = JObjectArray::from(self.env.new_local_ref(array)?);
        (0..self.env.get_array_length(&array)?)
            .map(|index| {
                let value = self.env.get_object_array_element(&array, index)?;
                self.owned(value)
            })
            .collect()
    }
    pub fn boxed_int(&mut self, value: i32) -> Result<O> {
        self.static_obj(
            "java/lang/Integer",
            "valueOf",
            "(I)Ljava/lang/Integer;",
            &[A::I(value)],
        )
    }
    pub fn boxed_bool(&mut self, value: bool) -> Result<O> {
        self.static_obj(
            "java/lang/Boolean",
            "valueOf",
            "(Z)Ljava/lang/Boolean;",
            &[A::Z(value)],
        )
    }
    pub fn id(&mut self, object: &O) -> Result<i64> {
        Ok(self.env.get_field(object, "nativeId", "J")?.j()?)
    }
    pub fn same(&self, a: &O, b: &O) -> Result<bool> {
        Ok(self.env.is_same_object(a, b)?)
    }
    pub fn application(&mut self) -> Result<O> {
        self.static_obj(
            "com/intellij/openapi/application/ApplicationManager",
            "getApplication",
            "()Lcom/intellij/openapi/application/Application;",
            &[],
        )
    }
}

type Handler = dyn for<'a> Fn(&mut J<'a>, &str, &[O]) -> Result<O> + Send + Sync;
static NEXT: AtomicI64 = AtomicI64::new(1);
static HANDLERS: OnceLock<Mutex<HashMap<i64, Arc<Handler>>>> = OnceLock::new();
fn handlers() -> &'static Mutex<HashMap<i64, Arc<Handler>>> {
    HANDLERS.get_or_init(Mutex::default)
}
pub fn register(
    handler: impl for<'a> Fn(&mut J<'a>, &str, &[O]) -> Result<O> + Send + Sync + 'static,
) -> i64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    handlers()
        .lock()
        .expect("callback registry poisoned")
        .insert(id, Arc::new(handler));
    id
}
pub fn unregister(id: i64) {
    // Dropping a callback can dispose its captured Scope and unregister other
    // callbacks. Release the registry lock before running those destructors.
    let removed = handlers()
        .lock()
        .expect("callback registry poisoned")
        .remove(&id);
    drop(removed);
}
pub fn invoke(j: &mut J<'_>, id: i64, operation: &str, args: &[O]) -> Result<O> {
    let handler = handlers()
        .lock()
        .expect("callback registry poisoned")
        .get(&id)
        .cloned();
    match handler {
        Some(handler) => handler(j, operation, args),
        None => j.null(),
    }
}
pub fn callback(j: &mut J<'_>, id: i64) -> Result<O> {
    j.new("dev/cranpose/rust/Callback", "(J)V", &[A::J(id)])
}
/// Every listener and native state handle belongs to a project/editor disposal scope.
#[derive(Default)]
pub struct Scope {
    ids: Mutex<Vec<i64>>,
}
impl Scope {
    pub fn register(
        &self,
        handler: impl for<'a> Fn(&mut J<'a>, &str, &[O]) -> Result<O> + Send + Sync + 'static,
    ) -> i64 {
        let id = register(handler);
        self.ids.lock().expect("scope poisoned").push(id);
        id
    }
    pub fn clear(&self) {
        let ids = std::mem::take(&mut *self.ids.lock().expect("scope poisoned"));
        for id in ids {
            unregister(id);
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        self.clear();
    }
}
pub fn later(
    j: &mut J<'_>,
    task: impl for<'a> Fn(&mut J<'a>) -> Result<()> + Send + Sync + 'static,
) -> Result<()> {
    let id_cell = Arc::new(AtomicI64::new(0));
    let captured = id_cell.clone();
    let id = register(move |j, _, _| {
        unregister(captured.load(Ordering::Acquire));
        task(j)?;
        j.null()
    });
    id_cell.store(id, Ordering::Release);
    let callback = callback(j, id)?;
    let app = j.application()?;
    if let Err(error) = j.void(
        &app,
        "invokeLater",
        "(Ljava/lang/Runnable;)V",
        &[A::O(&callback)],
    ) {
        unregister(id);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_destructors_can_unregister_other_callbacks() {
        struct Remove(i64);
        impl Drop for Remove {
            fn drop(&mut self) {
                unregister(self.0);
            }
        }
        let child = register(|j, _, _| j.null());
        let remove = Remove(child);
        let parent = register(move |j, _, _| {
            let _keep = &remove;
            j.null()
        });
        unregister(parent);
        assert!(!handlers().lock().expect("registry").contains_key(&child));
    }
}
