//! The plugin host is Rust. JVM extension adapters are generated during packaging.
mod tasks;
use jni::{
    JNIEnv,
    objects::{JClass, JObject, JObjectArray, JString},
    sys::jobject,
};

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_cranpose_rust_Native_call<'local>(
    env: JNIEnv<'local>,
    class: JClass<'local>,
    operation: JString<'local>,
    receiver: JObject<'local>,
    arguments: JObjectArray<'local>,
) -> jobject {
    cranpose_host::configure(cranpose_host::HostFeatures {
        cargo: false,
        stability: false,
    });
    cranpose_host::set_message_handler(tasks::handle);
    cranpose_host::Java_dev_cranpose_rust_Native_call(env, class, operation, receiver, arguments)
}
