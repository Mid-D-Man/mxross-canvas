// crates/mxross-android-media/src/lib.rs
//! Writes bytes into Android's MediaStore so exported files show up in
//! the Gallery app and any normal file manager.
//!
//! ## Verified API constraints (jni 0.22.4 source)
//!
//! - `JavaVM::from_raw` returns `Self` (not `Result`) — no `.map_err()`.
//! - `attach_current_thread` requires `E: From<jni::errors::Error>` —
//!   `String` does NOT satisfy this. A local `MediaError` type below
//!   bridges both JNI errors (via `From<jni::errors::Error>`) and our
//!   own descriptive messages, then converts to `String` after the
//!   closure returns.
//! - Every `AsRef<JNIStr>` param (find_class, call_method name, etc.)
//!   needs `jni_str!("...")` — plain `&str` / `&CStr` do NOT satisfy
//!   that trait.
//! - `RuntimeMethodSignature` needs `.method_signature()` before being
//!   passed to `call_method` / `call_static_method` / `new_object`.
//! - `RuntimeFieldSignature` needs `.field_signature()` before being
//!   passed to `get_static_field`.
//! - `JObject::from_raw` takes `(&mut env, raw)` not just `(raw)`.
//! - Inner closures that capture `env` mutably conflict with the outer
//!   `attach_current_thread` closure — everything is inlined flat.
//!
//! ## Scope limitation
//!
//! `RELATIVE_PATH` only exists on API 29+. `min_sdk_version = 26` means
//! API 26–28 devices will get an error string, not a working export.
//! The real test hardware (Samsung A13) is API 33 so this isn't blocking.

#[cfg(target_os = "android")]
mod android_impl {
    use jni::objects::{JObject, JValue};
    use jni::signature::{RuntimeFieldSignature, RuntimeMethodSignature};
    use jni::{jni_str, JavaVM};

    /// Local error type satisfying `E: From<jni::errors::Error>` as
    /// required by `attach_current_thread`. Converts to `String` at the
    /// public boundary.
    enum MediaError {
        Jni(jni::errors::Error),
        Msg(String),
    }

    impl From<jni::errors::Error> for MediaError {
        fn from(e: jni::errors::Error) -> Self {
            MediaError::Jni(e)
        }
    }

    impl MediaError {
        fn into_string(self) -> String {
            match self {
                MediaError::Jni(e) => format!("JNI error: {e}"),
                MediaError::Msg(s) => s,
            }
        }
    }

    macro_rules! msg {
        ($($arg:tt)*) => {
            MediaError::Msg(format!($($arg)*))
        };
    }

    pub fn save_png_to_pictures(
        display_name: &str,
        relative_subdir: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let ctx = ndk_context::android_context();
        // SAFETY: android-activity initializes the VM pointer before
        // android_main ever runs — this is documented and safe to call
        // from anywhere after that point.
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
        let activity_raw = ctx.context() as jni::sys::jobject;

        vm.attach_current_thread(|env| -> Result<(), MediaError> {
            // ── ContentValues values = new ContentValues() ────────────
            let cv_class = env
                .find_class(jni_str!("android/content/ContentValues"))?;
            let new_sig = RuntimeMethodSignature::from_str("()V")
                .map_err(|e| msg!("bad sig ()V: {e}"))?;
            let values = env.new_object(
                &cv_class,
                new_sig.method_signature(),
                &[],
            )?;

            // ── Helper sigs ───────────────────────────────────────────
            let put_ss_sig = RuntimeMethodSignature::from_str(
                "(Ljava/lang/String;Ljava/lang/String;)V",
            )
            .map_err(|e| msg!("bad sig put_ss: {e}"))?;

            let put_int_sig = RuntimeMethodSignature::from_str(
                "(Ljava/lang/String;Ljava/lang/Integer;)V",
            )
            .map_err(|e| msg!("bad sig put_int: {e}"))?;

            let void_sig = RuntimeMethodSignature::from_str("()V")
                .map_err(|e| msg!("bad sig ()V: {e}"))?;

            let value_of_sig =
                RuntimeMethodSignature::from_str("(I)Ljava/lang/Integer;")
                    .map_err(|e| msg!("bad sig valueOf: {e}"))?;

            let integer_class =
                env.find_class(jni_str!("java/lang/Integer"))?;

            // ── values.put(key, str) ×3 ───────────────────────────────
            let jdn = env.new_string(display_name)?;
            let k_dn = env.new_string("_display_name")?;
            env.call_method(
                &values,
                jni_str!("put"),
                put_ss_sig.method_signature(),
                &[JValue::Object(&k_dn), JValue::Object(&jdn)],
            )?;

            let jmt = env.new_string("image/png")?;
            let k_mt = env.new_string("mime_type")?;
            env.call_method(
                &values,
                jni_str!("put"),
                put_ss_sig.method_signature(),
                &[JValue::Object(&k_mt), JValue::Object(&jmt)],
            )?;

            let rel = format!("Pictures/{relative_subdir}");
            let jrp = env.new_string(&rel)?;
            let k_rp = env.new_string("relative_path")?;
            env.call_method(
                &values,
                jni_str!("put"),
                put_ss_sig.method_signature(),
                &[JValue::Object(&k_rp), JValue::Object(&jrp)],
            )?;

            // ── values.put("is_pending", Integer.valueOf(1)) ──────────
            let boxed_one = env
                .call_static_method(
                    &integer_class,
                    jni_str!("valueOf"),
                    value_of_sig.method_signature(),
                    &[JValue::Int(1)],
                )?
                .l()?;
            let k_ip = env.new_string("is_pending")?;
            env.call_method(
                &values,
                jni_str!("put"),
                put_int_sig.method_signature(),
                &[JValue::Object(&k_ip), JValue::Object(&boxed_one)],
            )?;

            // ── resolver = activity.getContentResolver() ──────────────
            let activity_obj = unsafe { JObject::from_raw(env, activity_raw) };
            let get_resolver_sig = RuntimeMethodSignature::from_str(
                "()Landroid/content/ContentResolver;",
            )
            .map_err(|e| msg!("bad sig getContentResolver: {e}"))?;
            let resolver = env
                .call_method(
                    &activity_obj,
                    jni_str!("getContentResolver"),
                    get_resolver_sig.method_signature(),
                    &[],
                )?
                .l()?;

            // ── collection = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
            let media_class = env.find_class(
                jni_str!("android/provider/MediaStore$Images$Media"),
            )?;
            let uri_field_sig =
                RuntimeFieldSignature::from_str("Landroid/net/Uri;")
                    .map_err(|e| msg!("bad field sig: {e}"))?;
            let collection = env
                .get_static_field(
                    &media_class,
                    jni_str!("EXTERNAL_CONTENT_URI"),
                    uri_field_sig.field_signature(),
                )?
                .l()?;

            // ── item = resolver.insert(collection, values) ────────────
            let insert_sig = RuntimeMethodSignature::from_str(
                "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;",
            )
            .map_err(|e| msg!("bad sig insert: {e}"))?;
            let item = env
                .call_method(
                    &resolver,
                    jni_str!("insert"),
                    insert_sig.method_signature(),
                    &[
                        JValue::Object(&collection),
                        JValue::Object(&values),
                    ],
                )?
                .l()?;
            if item.is_null() {
                return Err(msg!(
                    "ContentResolver.insert returned null — MediaStore rejected the request"
                ));
            }

            // ── os = resolver.openOutputStream(item) ──────────────────
            let open_sig = RuntimeMethodSignature::from_str(
                "(Landroid/net/Uri;)Ljava/io/OutputStream;",
            )
            .map_err(|e| msg!("bad sig openOutputStream: {e}"))?;
            let stream = env
                .call_method(
                    &resolver,
                    jni_str!("openOutputStream"),
                    open_sig.method_signature(),
                    &[JValue::Object(&item)],
                )?
                .l()?;

            // ── os.write(bytes); os.close() ───────────────────────────
            let jbytes = env.byte_array_from_slice(bytes)?;
            let write_sig = RuntimeMethodSignature::from_str("([B)V")
                .map_err(|e| msg!("bad sig write: {e}"))?;
            env.call_method(
                &stream,
                jni_str!("write"),
                write_sig.method_signature(),
                &[JValue::Object(&jbytes)],
            )?;
            env.call_method(
                &stream,
                jni_str!("close"),
                void_sig.method_signature(),
                &[],
            )?;

            // ── resolver.update to clear IS_PENDING ───────────────────
            env.call_method(
                &values,
                jni_str!("clear"),
                void_sig.method_signature(),
                &[],
            )?;
            let boxed_zero = env
                .call_static_method(
                    &integer_class,
                    jni_str!("valueOf"),
                    value_of_sig.method_signature(),
                    &[JValue::Int(0)],
                )?
                .l()?;
            let k_ip2 = env.new_string("is_pending")?;
            env.call_method(
                &values,
                jni_str!("put"),
                put_int_sig.method_signature(),
                &[JValue::Object(&k_ip2), JValue::Object(&boxed_zero)],
            )?;

            let update_sig = RuntimeMethodSignature::from_str(
                "(Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;)I",
            )
            .map_err(|e| msg!("bad sig update: {e}"))?;
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
            )?;

            Ok(())
        })
        .map_err(|e: MediaError| e.into_string())
    }
}

#[cfg(target_os = "android")]
pub use android_impl::save_png_to_pictures;

/// Non-Android stub so mxross-android doesn't need its own cfg gate
/// at every call site.
#[cfg(not(target_os = "android"))]
pub fn save_png_to_pictures(
    _display_name: &str,
    _relative_subdir: &str,
    _bytes: &[u8],
) -> Result<(), String> {
    Err("mxross-android-media is only implemented for Android".to_string())
}
