// crates/mxross-android-media/src/lib.rs
//! Writes bytes into Android's MediaStore (the Pictures/Movies/etc.
//! gallery index), so exported files actually show up in the Gallery
//! app and any normal file manager — unlike
//! `external_data_path()/exports/...`, which is real storage but lives
//! in the app-private, hidden `Android/data/<package>/` sandbox.
//!
//! ## Scope limitation, stated plainly
//!
//! This uses the `MediaStore.Images.Media.RELATIVE_PATH` column, which
//! only exists on API 29+ (Android 10, "Scoped Storage"). The app's
//! manifest currently allows `min_sdk_version = 26`, so there's a real
//! gap: a device running Android 8.0–9.0 (API 26–28) would hit this
//! path and get a JNI exception (`NoSuchFieldError` looking up a field
//! that doesn't exist on that API level) rather than a working export.
//! Not silently swallowed — `save_png_to_pictures` below returns `Err`
//! rather than panicking, but it genuinely won't work pre-API-29 as
//! written. Worth raising `min_sdk_version` to 29 in
//! `mxross-android/Cargo.toml`, or adding a legacy
//! `WRITE_EXTERNAL_STORAGE` + direct-`File` fallback branch, once that
//! range of devices actually matters — not done here since the real
//! test hardware (Samsung A13) is API 33.
//!
//! ## Why this is its own crate
//!
//! First JNI in the whole MxRoss Canvas codebase. Isolating it here
//! means `mxross-export` (PNG/MPX encoding) stays a pure data
//! transformation with zero Android/JNI knowledge — this crate's only
//! job is "take already-encoded bytes, hand them to MediaStore."
//! `target_os = "android"`-gated end to end so it's a guaranteed no-op
//! dependency on any other platform.

#[cfg(target_os = "android")]
mod android_impl {
    use jni::objects::{JObject, JValue};
    use jni::signature::RuntimeMethodSignature;
    use jni::JavaVM;

    /// Writes `bytes` into `MediaStore.Images.Media`, named `display_name`
    /// (e.g. `"canvas.png"`), under `Pictures/<relative_subdir>` (e.g.
    /// `"MxRoss"` -> `Pictures/MxRoss/canvas.png`, matching the Gallery's
    /// usual per-app folder convention). Returns `Err` with a human-
    /// readable message on any failure rather than panicking — a failed
    /// export shouldn't be able to take the whole app down.
    pub fn save_png_to_pictures(
        display_name: &str,
        relative_subdir: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let ctx = ndk_context::android_context();
        // SAFETY: android-activity has already initialized this VM
        // pointer (and the Activity/Context pointer below) before
        // android_main ever runs — that's the whole reason
        // ndk_context::android_context() is documented as safe to call
        // from anywhere in the app.
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|e| format!("JavaVM::from_raw failed: {e}"))?;
        let activity = ctx.context() as jni::sys::jobject;

        vm.attach_current_thread(|env| -> Result<(), String> {
            // ContentValues values = new ContentValues();
            let content_values_class = env
                .find_class(c"android/content/ContentValues")
                .map_err(|e| format!("find_class(ContentValues) failed: {e}"))?;
            let new_ctor_sig = RuntimeMethodSignature::from_str("()V")
                .map_err(|e| format!("bad signature: {e}"))?;
            let values = env
                .new_object(&content_values_class, &new_ctor_sig, &[])
                .map_err(|e| format!("new ContentValues() failed: {e}"))?;

            let put_str_sig = RuntimeMethodSignature::from_str(
                "(Ljava/lang/String;Ljava/lang/String;)V",
            )
            .map_err(|e| format!("bad signature: {e}"))?;

            let put_string = |env: &mut jni::Env, key: &str, value: &str| -> Result<(), String> {
                let jkey = env.new_string(key).map_err(|e| format!("new_string({key}) failed: {e}"))?;
                let jvalue = env
                    .new_string(value)
                    .map_err(|e| format!("new_string({value}) failed: {e}"))?;
                env.call_method(
                    &values,
                    c"put",
                    &put_str_sig,
                    &[JValue::Object(&jkey), JValue::Object(&jvalue)],
                )
                .map_err(|e| format!("ContentValues.put({key}) failed: {e}"))?;
                Ok(())
            };

            // values.put(DISPLAY_NAME, display_name);
            put_string(env, "_display_name", display_name)?;
            // values.put(MIME_TYPE, "image/png");
            put_string(env, "mime_type", "image/png")?;
            // values.put(RELATIVE_PATH, "Pictures/<relative_subdir>");
            put_string(env, "relative_path", &format!("Pictures/{relative_subdir}"))?;

            // values.put(IS_PENDING, 1);
            let put_int_sig = RuntimeMethodSignature::from_str("(Ljava/lang/String;Ljava/lang/Integer;)V")
                .map_err(|e| format!("bad signature: {e}"))?;
            // IS_PENDING wants a boxed Integer, not a primitive int —
            // ContentValues has no put(String, int) overload, only
            // put(String, Integer); Integer.valueOf(1) boxes it.
            let integer_class = env
                .find_class(c"java/lang/Integer")
                .map_err(|e| format!("find_class(Integer) failed: {e}"))?;
            let value_of_sig = RuntimeMethodSignature::from_str("(I)Ljava/lang/Integer;")
                .map_err(|e| format!("bad signature: {e}"))?;
            let boxed_one = env
                .call_static_method(&integer_class, c"valueOf", &value_of_sig, &[JValue::Int(1)])
                .map_err(|e| format!("Integer.valueOf(1) failed: {e}"))?
                .l()
                .map_err(|e| format!("Integer.valueOf(1) wasn't an object: {e}"))?;
            let key_is_pending = env
                .new_string("is_pending")
                .map_err(|e| format!("new_string(is_pending) failed: {e}"))?;
            env.call_method(
                &values,
                c"put",
                &put_int_sig,
                &[JValue::Object(&key_is_pending), JValue::Object(&boxed_one)],
            )
            .map_err(|e| format!("ContentValues.put(is_pending) failed: {e}"))?;

            // ContentResolver resolver = activityContext.getContentResolver();
            let get_resolver_sig =
                RuntimeMethodSignature::from_str("()Landroid/content/ContentResolver;")
                    .map_err(|e| format!("bad signature: {e}"))?;
            let activity_obj = unsafe { JObject::from_raw(activity) };
            let resolver = env
                .call_method(&activity_obj, c"getContentResolver", &get_resolver_sig, &[])
                .map_err(|e| format!("getContentResolver() failed: {e}"))?
                .l()
                .map_err(|e| format!("getContentResolver() wasn't an object: {e}"))?;

            // Uri collection = MediaStore.Images.Media.EXTERNAL_CONTENT_URI;
            let media_images_class = env
                .find_class(c"android/provider/MediaStore$Images$Media")
                .map_err(|e| format!("find_class(MediaStore.Images.Media) failed: {e}"))?;
            let uri_field_sig = RuntimeMethodSignature::from_str("Landroid/net/Uri;")
                .map_err(|e| format!("bad field signature: {e}"))?;
            // get_static_field's signature parameter is a FIELD
            // signature here, not a method one — verified directly
            // against env.rs's get_static_field signature.
            let collection = env
                .get_static_field(
                    &media_images_class,
                    "EXTERNAL_CONTENT_URI",
                    jni::signature::RuntimeFieldSignature::from_str("Landroid/net/Uri;")
                        .map_err(|e| format!("bad field signature: {e}"))?,
                )
                .map_err(|e| format!("EXTERNAL_CONTENT_URI lookup failed: {e}"))?
                .l()
                .map_err(|e| format!("EXTERNAL_CONTENT_URI wasn't an object: {e}"))?;

            // Uri item = resolver.insert(collection, values);
            let insert_sig = RuntimeMethodSignature::from_str(
                "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;",
            )
            .map_err(|e| format!("bad signature: {e}"))?;
            let item = env
                .call_method(
                    &resolver,
                    c"insert",
                    &insert_sig,
                    &[JValue::Object(&collection), JValue::Object(&values)],
                )
                .map_err(|e| format!("ContentResolver.insert(...) failed: {e}"))?
                .l()
                .map_err(|e| format!("insert(...) returned no object: {e}"))?;
            if item.is_null() {
                return Err("ContentResolver.insert(...) returned null — MediaStore rejected the request".to_string());
            }

            // OutputStream os = resolver.openOutputStream(item);
            let open_stream_sig =
                RuntimeMethodSignature::from_str("(Landroid/net/Uri;)Ljava/io/OutputStream;")
                    .map_err(|e| format!("bad signature: {e}"))?;
            let stream = env
                .call_method(&resolver, c"openOutputStream", &open_stream_sig, &[JValue::Object(&item)])
                .map_err(|e| format!("openOutputStream(...) failed: {e}"))?
                .l()
                .map_err(|e| format!("openOutputStream(...) returned no object: {e}"))?;

            // os.write(byte[]); os.close();
            let jbytes = env
                .byte_array_from_slice(bytes)
                .map_err(|e| format!("byte_array_from_slice failed: {e}"))?;
            let write_sig = RuntimeMethodSignature::from_str("([B)V")
                .map_err(|e| format!("bad signature: {e}"))?;
            env.call_method(&stream, c"write", &write_sig, &[JValue::Object(&jbytes)])
                .map_err(|e| format!("OutputStream.write(byte[]) failed: {e}"))?;
            let close_sig = RuntimeMethodSignature::from_str("()V")
                .map_err(|e| format!("bad signature: {e}"))?;
            env.call_method(&stream, c"close", &close_sig, &[])
                .map_err(|e| format!("OutputStream.close() failed: {e}"))?;

            // values.clear(); values.put(IS_PENDING, 0); resolver.update(item, values, null, null);
            let clear_sig = RuntimeMethodSignature::from_str("()V")
                .map_err(|e| format!("bad signature: {e}"))?;
            env.call_method(&values, c"clear", &clear_sig, &[])
                .map_err(|e| format!("ContentValues.clear() failed: {e}"))?;
            let boxed_zero = env
                .call_static_method(&integer_class, c"valueOf", &value_of_sig, &[JValue::Int(0)])
                .map_err(|e| format!("Integer.valueOf(0) failed: {e}"))?
                .l()
                .map_err(|e| format!("Integer.valueOf(0) wasn't an object: {e}"))?;
            let key_is_pending2 = env
                .new_string("is_pending")
                .map_err(|e| format!("new_string(is_pending) failed: {e}"))?;
            env.call_method(
                &values,
                c"put",
                &put_int_sig,
                &[JValue::Object(&key_is_pending2), JValue::Object(&boxed_zero)],
            )
            .map_err(|e| format!("ContentValues.put(is_pending=0) failed: {e}"))?;

            let update_sig = RuntimeMethodSignature::from_str(
                "(Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;)I",
            )
            .map_err(|e| format!("bad signature: {e}"))?;
            env.call_method(
                &resolver,
                c"update",
                &update_sig,
                &[
                    JValue::Object(&item),
                    JValue::Object(&values),
                    JValue::Object(&JObject::null()),
                    JValue::Object(&JObject::null()),
                ],
            )
            .map_err(|e| format!("ContentResolver.update(...) failed: {e}"))?;

            Ok(())
        })
        .map_err(|e: String| e)
    }
}

#[cfg(target_os = "android")]
pub use android_impl::save_png_to_pictures;

/// Non-Android stub — always errors rather than failing to compile, so
/// `mxross-android` (the only consumer) doesn't need its own
/// `cfg(target_os = "android")` gate at every call site. There is no
/// real desktop/iOS equivalent of "save to the Android Gallery"; a
/// future desktop platform crate would call straight into
/// `mxross-export` and write a plain file instead, bypassing this crate
/// entirely.
#[cfg(not(target_os = "android"))]
pub fn save_png_to_pictures(_display_name: &str, _relative_subdir: &str, _bytes: &[u8]) -> Result<(), String> {
    Err("mxross-android-media is only implemented for Android".to_string())
}
