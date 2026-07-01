// crates/mxross-android-media/src/lib.rs
//! Writes bytes into Android's MediaStore (the Pictures/Movies/etc.
//! gallery index), so exported files actually show up in the Gallery
//! app and any normal file manager — unlike
//! `external_data_path()/exports/...`, which is real storage but lives
//! in the app-private, hidden `Android/data/<package>/` sandbox.
//!
//! ## API notes (verified against jni 0.22.4 source directly)
//!
//! - `JavaVM::from_raw` returns `Self` directly, NOT `Result` — no
//!   `.map_err()` needed; `assert!(!ptr.is_null())` is its only check.
//! - Method names require `jni_str!("name")` which gives `&JNIStr`,
//!   since `CStr` does NOT implement `AsRef<JNIStr>` despite the
//!   `c"..."` literal syntax looking similar.
//! - `RuntimeMethodSignature` does NOT implement `AsRef<MethodSignature>`
//!   directly — you must call `.method_signature()` on it first, which
//!   does implement that trait.
//! - `JObject::from_raw` takes `(&env, raw)` not just `(raw)`.
//! - `get_static_field` takes `S: AsRef<FieldSignature>` — use
//!   `RuntimeFieldSignature::from_str(...)?` then `.field_signature()`.
//!
//! ## Scope limitation, stated plainly
//!
//! `MediaStore.Images.Media.RELATIVE_PATH` only exists on API 29+
//! (Android 10, "Scoped Storage"). The app currently allows
//! `min_sdk_version = 26`, so devices on API 26–28 will get a JNI
//! error from this function rather than a working export. The real test
//! hardware (Samsung A13) is API 33, so this isn't a blocking problem
//! right now — raise `min_sdk_version` to 29 in
//! `mxross-android/Cargo.toml` when/if that gap actually matters.

#[cfg(target_os = "android")]
mod android_impl {
    use jni::objects::{JObject, JValue};
    use jni::signature::{RuntimeFieldSignature, RuntimeMethodSignature};
    use jni::{jni_str, JavaVM};

    pub fn save_png_to_pictures(
        display_name: &str,
        relative_subdir: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let ctx = ndk_context::android_context();
        // SAFETY: android-activity has already initialized this VM
        // pointer before android_main runs.
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };

        let activity_raw = ctx.context() as jni::sys::jobject;

        vm.attach_current_thread(|env| -> Result<(), String> {
            // --- Build ContentValues ---
            let cv_class = env
                .find_class("android/content/ContentValues")
                .map_err(|e| format!("find ContentValues: {e}"))?;
            let new_sig = RuntimeMethodSignature::from_str("()V")
                .map_err(|e| format!("sig: {e}"))?;
            let values = env
                .new_object(&cv_class, new_sig.method_signature(), &[])
                .map_err(|e| format!("new ContentValues: {e}"))?;

            let put_ss_sig = RuntimeMethodSignature::from_str(
                "(Ljava/lang/String;Ljava/lang/String;)V",
            )
            .map_err(|e| format!("sig: {e}"))?;
            let put_ss = put_ss_sig.method_signature();

            let put_str = |env: &mut jni::Env,
                           key: &str,
                           val: &str|
             -> Result<(), String> {
                let jk = env
                    .new_string(key)
                    .map_err(|e| format!("new_string({key}): {e}"))?;
                let jv = env
                    .new_string(val)
                    .map_err(|e| format!("new_string({val}): {e}"))?;
                env.call_method(
                    &values,
                    jni_str!("put"),
                    put_ss,
                    &[JValue::Object(&jk), JValue::Object(&jv)],
                )
                .map_err(|e| format!("put({key}): {e}"))?;
                Ok(())
            };

            put_str(env, "_display_name", display_name)?;
            put_str(env, "mime_type", "image/png")?;
            put_str(env, "relative_path", &format!("Pictures/{relative_subdir}"))?;

            // IS_PENDING = 1  (boxed Integer, not primitive int)
            let integer_class = env
                .find_class("java/lang/Integer")
                .map_err(|e| format!("find Integer: {e}"))?;
            let value_of_sig = RuntimeMethodSignature::from_str("(I)Ljava/lang/Integer;")
                .map_err(|e| format!("sig: {e}"))?;
            let put_int_sig = RuntimeMethodSignature::from_str(
                "(Ljava/lang/String;Ljava/lang/Integer;)V",
            )
            .map_err(|e| format!("sig: {e}"))?;

            let set_is_pending = |env: &mut jni::Env, flag: i32| -> Result<(), String> {
                let boxed = env
                    .call_static_method(
                        &integer_class,
                        jni_str!("valueOf"),
                        value_of_sig.method_signature(),
                        &[JValue::Int(flag)],
                    )
                    .map_err(|e| format!("Integer.valueOf({flag}): {e}"))?
                    .l()
                    .map_err(|e| format!("valueOf not object: {e}"))?;
                let jk = env
                    .new_string("is_pending")
                    .map_err(|e| format!("new_string: {e}"))?;
                env.call_method(
                    &values,
                    jni_str!("put"),
                    put_int_sig.method_signature(),
                    &[JValue::Object(&jk), JValue::Object(&boxed)],
                )
                .map_err(|e| format!("put(is_pending={flag}): {e}"))?;
                Ok(())
            };
            set_is_pending(env, 1)?;

            // --- resolver = activity.getContentResolver() ---
            let activity_obj = unsafe { JObject::from_raw(env, activity_raw) };
            let get_resolver_sig = RuntimeMethodSignature::from_str(
                "()Landroid/content/ContentResolver;",
            )
            .map_err(|e| format!("sig: {e}"))?;
            let resolver = env
                .call_method(
                    &activity_obj,
                    jni_str!("getContentResolver"),
                    get_resolver_sig.method_signature(),
                    &[],
                )
                .map_err(|e| format!("getContentResolver: {e}"))?
                .l()
                .map_err(|e| format!("resolver not object: {e}"))?;

            // --- collection = MediaStore.Images.Media.EXTERNAL_CONTENT_URI ---
            let media_class = env
                .find_class("android/provider/MediaStore$Images$Media")
                .map_err(|e| format!("find MediaStore.Images.Media: {e}"))?;
            let uri_field_sig = RuntimeFieldSignature::from_str("Landroid/net/Uri;")
                .map_err(|e| format!("field sig: {e}"))?;
            let collection = env
                .get_static_field(
                    &media_class,
                    jni_str!("EXTERNAL_CONTENT_URI"),
                    uri_field_sig.field_signature(),
                )
                .map_err(|e| format!("EXTERNAL_CONTENT_URI: {e}"))?
                .l()
                .map_err(|e| format!("uri not object: {e}"))?;

            // --- item = resolver.insert(collection, values) ---
            let insert_sig = RuntimeMethodSignature::from_str(
                "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;",
            )
            .map_err(|e| format!("sig: {e}"))?;
            let item = env
                .call_method(
                    &resolver,
                    jni_str!("insert"),
                    insert_sig.method_signature(),
                    &[JValue::Object(&collection), JValue::Object(&values)],
                )
                .map_err(|e| format!("insert: {e}"))?
                .l()
                .map_err(|e| format!("item not object: {e}"))?;
            if item.is_null() {
                return Err(
                    "ContentResolver.insert returned null — MediaStore rejected the request"
                        .to_string(),
                );
            }

            // --- os = resolver.openOutputStream(item) ---
            let open_sig = RuntimeMethodSignature::from_str(
                "(Landroid/net/Uri;)Ljava/io/OutputStream;",
            )
            .map_err(|e| format!("sig: {e}"))?;
            let stream = env
                .call_method(
                    &resolver,
                    jni_str!("openOutputStream"),
                    open_sig.method_signature(),
                    &[JValue::Object(&item)],
                )
                .map_err(|e| format!("openOutputStream: {e}"))?
                .l()
                .map_err(|e| format!("stream not object: {e}"))?;

            // --- os.write(bytes); os.close() ---
            let jbytes = env
                .byte_array_from_slice(bytes)
                .map_err(|e| format!("byte_array_from_slice: {e}"))?;
            let write_sig = RuntimeMethodSignature::from_str("([B)V")
                .map_err(|e| format!("sig: {e}"))?;
            env.call_method(
                &stream,
                jni_str!("write"),
                write_sig.method_signature(),
                &[JValue::Object(&jbytes)],
            )
            .map_err(|e| format!("write: {e}"))?;

            let close_sig = RuntimeMethodSignature::from_str("()V")
                .map_err(|e| format!("sig: {e}"))?;
            env.call_method(
                &stream,
                jni_str!("close"),
                close_sig.method_signature(),
                &[],
            )
            .map_err(|e| format!("close: {e}"))?;

            // --- resolver.update(item, values, null, null) to publish ---
            let clear_sig = RuntimeMethodSignature::from_str("()V")
                .map_err(|e| format!("sig: {e}"))?;
            env.call_method(&values, jni_str!("clear"), clear_sig.method_signature(), &[])
                .map_err(|e| format!("clear: {e}"))?;
            set_is_pending(env, 0)?;

            let update_sig = RuntimeMethodSignature::from_str(
                "(Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;)I",
            )
            .map_err(|e| format!("sig: {e}"))?;
            env.call_method(
                &resolver,
                jni_str!("update"),
                update_sig.method_signature(),
                &[
                    JValue::Object(&item),
                    JValue::Object(&values),
                    JValue::Object(&JObject::null()),
                    JValue::Object(&JObject::null()),
                ],
            )
            .map_err(|e| format!("update: {e}"))?;

            Ok(())
        })
        .map_err(|e: String| e)
    }
}

#[cfg(target_os = "android")]
pub use android_impl::save_png_to_pictures;

/// Non-Android stub — always errors rather than failing to compile, so
/// `mxross-android` doesn't need its own `cfg` gate at every call site.
#[cfg(not(target_os = "android"))]
pub fn save_png_to_pictures(
    _display_name: &str,
    _relative_subdir: &str,
    _bytes: &[u8],
) -> Result<(), String> {
    Err("mxross-android-media is only implemented for Android".to_string())
                    }
