//! JNI entrypoints for `GalleryActivity`, which owns one gallery session.
//!
//! These are independent of the plugin bridge: the Activity hands Rust its
//! `WidgetHost`, and Rust hands back the native root to mount.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use leptoskit::android::AndroidBackend;
use leptoskit::android::jni::JNIEnv;
use leptoskit::android::jni::objects::JObject;
use leptoskit::android::jni::objects::JString;
use leptoskit::android::jni::objects::JValue;
use leptoskit::android::jni::sys::jfloat;
use leptoskit::android::jni::sys::jlong;

use crate::Assets;
use crate::native::NativeGallery;
use crate::native::screen;

type Session = Rc<RefCell<NativeGallery<AndroidBackend>>>;

/// The live sessions, keyed by the opaque handle each Activity holds.
#[derive(Default)]
struct Activities {
	next: jlong,
	sessions: BTreeMap<jlong, Session>,
}

thread_local! {
	static ACTIVITIES: RefCell<Activities> = RefCell::new(Activities::default());
}

fn session(handle: jlong) -> Result<Session, String> {
	ACTIVITIES
		.with(|activities| activities.borrow().sessions.get(&handle).cloned())
		.ok_or_else(|| "the gallery Activity has already been destroyed".into())
}

fn root(session: &Session) -> Result<jlong, String> {
	session
		.borrow()
		.surface
		.root()
		.map(|node| node.index() as jlong)
		.ok_or_else(|| "the screen committed no root".into())
}

fn string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Result<String, String> {
	env.get_string(value)
		.map(Into::into)
		.map_err(|error| error.to_string())
}

fn create(
	env: &mut JNIEnv<'_>,
	activity: &JObject<'_>,
	host: &JObject<'_>,
	viewport: (f32, f32),
	slug: &JString<'_>,
	asset_dir: &JString<'_>,
) -> Result<jlong, String> {
	let screen = screen(&string(env, slug)?)?;
	let assets = Assets::generate(string(env, asset_dir)?).map_err(|error| error.to_string())?;
	let backend = AndroidBackend::new(env, host).map_err(|error| error.to_string())?;
	let session = Rc::new(RefCell::new(NativeGallery::new(
		backend, screen, assets, viewport,
	)));
	let activity = env
		.new_global_ref(activity)
		.map_err(|error| error.to_string())?;
	let vm = env.get_java_vm().map_err(|error| error.to_string())?;
	let weak = Rc::downgrade(&session);
	// The host delivers events on the UI thread, outside any session borrow.
	session
		.borrow()
		.surface
		.backend()
		.set_event_listener(move |node, event| {
			let Some(session) = weak.upgrade() else {
				return;
			};
			let previous = root(&session).expect("a mounted gallery has a root");
			session.borrow_mut().dispatch(node, event);
			let current = root(&session).expect("a reconciled gallery has a root");
			if current != previous {
				let mut env = vm
					.get_env()
					.expect("widget callbacks run on the attached UI thread");
				env.call_method(
					activity.as_obj(),
					"mountRoot",
					"(J)V",
					&[JValue::Long(current)],
				)
				.expect("the live gallery Activity mounts its replacement root");
			}
		});

	ACTIVITIES.with(|activities| {
		let mut activities = activities.borrow_mut();
		activities.next = activities
			.next
			.checked_add(1)
			.ok_or("gallery Activity handles are exhausted")?;
		let handle = activities.next;
		activities.sessions.insert(handle, session);
		Ok(handle)
	})
}

/// Returns `result`'s value, or throws its error into the calling Activity.
fn throw_on_error(env: &mut JNIEnv<'_>, result: Result<jlong, String>) -> jlong {
	result.unwrap_or_else(|error| {
		env.throw_new("java/lang/IllegalStateException", error)
			.expect("the JVM accepts a gallery exception");
		0
	})
}

// SAFETY: GalleryActivity declares these exact signatures and calls them on
// its UI thread. Local references never outlive the call.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_gallery_GalleryActivity_nativeCreate(
	mut env: JNIEnv<'_>,
	activity: JObject<'_>,
	host: JObject<'_>,
	width: jfloat,
	height: jfloat,
	slug: JString<'_>,
	asset_dir: JString<'_>,
) -> jlong {
	let created = create(
		&mut env,
		&activity,
		&host,
		(width, height),
		&slug,
		&asset_dir,
	);
	throw_on_error(&mut env, created)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_gallery_GalleryActivity_nativeRoot(
	mut env: JNIEnv<'_>,
	_activity: JObject<'_>,
	handle: jlong,
) -> jlong {
	let root = session(handle).and_then(|session| root(&session));
	throw_on_error(&mut env, root)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_gallery_GalleryActivity_nativeResize(
	mut env: JNIEnv<'_>,
	_activity: JObject<'_>,
	handle: jlong,
	width: jfloat,
	height: jfloat,
) -> jlong {
	let resized = session(handle).and_then(|session| {
		session.borrow_mut().surface.set_viewport(width, height);
		root(&session)
	});
	throw_on_error(&mut env, resized)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_gallery_GalleryActivity_nativeDestroy(
	_env: JNIEnv<'_>,
	_activity: JObject<'_>,
	handle: jlong,
) {
	// Drop outside the registry borrow: detaching a focused native editor can
	// synchronously deliver a final focus callback to the backend.
	let session = ACTIVITIES.with(|activities| activities.borrow_mut().sessions.remove(&handle));
	drop(session);
}
