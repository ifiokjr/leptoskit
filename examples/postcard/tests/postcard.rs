//! The postcard read in every language, and changed the way a person would:
//! stamps switch the language, sliders change the counts, and taps change
//! the selects — over the real renderer on the headless backend.

use std::path::Path;

use leptoskit::native::Role;
use leptoskit::yoga::Direction;
use leptoskit_l10n::Locale;
use leptoskit_testing::Finder;
use leptoskit_testing::TestError;
use leptoskit_testing::WidgetTester;
use postcard_example::PostcardApp;
use postcard_example::VIEWPORT;
use reactive_graph::traits::Get;
use reactive_graph::traits::Set;

fn locale(tag: &str) -> Locale {
	Locale::new(tag).expect("a valid tag")
}

fn mount(tag: &str) -> Result<(PostcardApp, WidgetTester), TestError> {
	let app = PostcardApp::new(&[locale(tag)]);
	let render = app.clone();
	let tester = WidgetTester::new(move || render.render(), VIEWPORT.0, VIEWPORT.1)?;
	Ok((app, tester))
}

#[test]
fn the_committed_module_is_what_the_arb_files_generate() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR"));
	let generated = leptoskit_l10n_codegen::generate(&root.join("locales"), "en")
		.expect("the ARB files are valid");
	assert_eq!(
		leptoskit_l10n_codegen::status(&root.join("src/l10n.rs"), &generated)
			.expect("src/l10n.rs is readable"),
		leptoskit_l10n_codegen::Status::Current,
		"src/l10n.rs is out of date with locales/; run `leptoskit l10n` in examples/postcard"
	);
}

#[test]
fn each_language_starts_where_the_person_prefers() {
	for (preferred, chosen) in [
		("pl-PL", "pl"),
		("pt-PT", "pt-BR"),
		("ar-EG", "ar"),
		("ja-JP", "ja"),
		("de", "en"),
	] {
		let app = PostcardApp::new(&[locale(preferred)]);
		assert_eq!(app.l10n().locale(), chosen, "{preferred}");
	}
	assert_eq!(PostcardApp::new(&[]).l10n().locale(), "en");
}

#[test]
fn every_language_counts_the_days_its_own_way() {
	let cases: [(&str, &[(i64, &str)]); 5] = [
		(
			"en",
			&[
				(0, "Arrived today"),
				(1, "1 day away from home"),
				(2, "2 days away from home"),
			],
		),
		// Polish has four forms: 2–4 and 22–24 are few, 5–21 and 25 many.
		(
			"pl",
			&[
				(1, "1 dzień poza domem"),
				(2, "2 dni poza domem"),
				(5, "5 dni poza domem"),
				(22, "22 dni poza domem"),
			],
		),
		// Arabic has six, chosen by the rules, not by `=N`.
		(
			"ar",
			&[
				(0, "وصلت اليوم"),
				(1, "يوم واحد بعيدًا عن البيت"),
				(2, "يومان بعيدًا عن البيت"),
				(3, "3 أيام بعيدًا عن البيت"),
				(11, "11 يومًا بعيدًا عن البيت"),
				(100, "100 يوم بعيدًا عن البيت"),
			],
		),
		// Japanese has one form, and an exact match for zero.
		(
			"ja",
			&[
				(0, "今日到着しました"),
				(1, "家を離れて1日目"),
				(30, "家を離れて30日目"),
			],
		),
		(
			"pt-BR",
			&[(1, "1 dia longe de casa"), (2, "2 dias longe de casa")],
		),
	];

	for (tag, counts) in cases {
		let app = PostcardApp::new(&[locale(tag)]);
		let l10n = app.l10n();
		for &(days, expected) in counts {
			assert_eq!(l10n.days_away(days), expected, "{tag} at {days}");
		}
	}
}

#[test]
fn photos_switch_polish_forms_at_the_cldr_boundaries() {
	let app = PostcardApp::new(&[locale("pl")]);
	let l10n = app.l10n();
	assert_eq!(l10n.photos_taken(0), "Jeszcze bez zdjęć morza");
	assert_eq!(l10n.photos_taken(1), "Jedno zdjęcie morza");
	assert_eq!(l10n.photos_taken(4), "4 zdjęcia morza");
	assert_eq!(l10n.photos_taken(12), "12 zdjęć morza");
	assert_eq!(l10n.photos_taken(24), "24 zdjęcia morza");
	assert_eq!(l10n.photos_taken(25), "25 zdjęć morza");
}

#[test]
fn selects_pick_their_case_and_fall_back_to_other() {
	let app = PostcardApp::new(&[locale("en")]);
	let l10n = app.l10n();
	assert_eq!(l10n.weather("sunny"), "The sun has not stopped shining.");
	assert_eq!(l10n.weather("fog"), "The weather keeps changing its mind.");
	assert_eq!(l10n.sign_off("female"), "She sends her love");
	assert_eq!(l10n.sign_off("other"), "They send their love");
	// A plural nested in a select.
	assert_eq!(
		l10n.letters_waiting("you", 1),
		"You have 1 letter waiting at home"
	);
	assert_eq!(
		l10n.letters_waiting("Ana", 3),
		"Ana has 3 letters waiting at home"
	);

	let ja_app = PostcardApp::new(&[locale("ja")]);
	let ja = ja_app.l10n();
	assert_eq!(ja.sign_off("female"), "愛をこめて");
	assert_eq!(
		ja.letters_waiting("Ana", 2),
		"Anaさんの家に手紙が2通届いています"
	);
}

#[test]
fn a_partial_translation_falls_back_to_the_template() {
	let app = PostcardApp::new(&[locale("pt-BR")]);
	let l10n = app.l10n();
	assert_eq!(l10n.app_title(), "Cartão-postal");
	assert_eq!(l10n.weather("rainy"), "It has rained every afternoon.");
	assert_eq!(l10n.photos_label(), "Photos");

	let ar_app = PostcardApp::new(&[locale("ar")]);
	let ar = ar_app.l10n();
	assert_eq!(
		ar.letters_waiting("you", 2),
		"You have 2 letters waiting at home"
	);
}

#[test]
fn a_stamp_switches_every_message_on_screen() -> Result<(), TestError> {
	let (app, mut tester) = mount("en")?;
	tester.find(&Finder::text("Greetings from Lisbon!"))?;
	assert_eq!(
		tester
			.find(&Finder::aria_role(Role::Button, "en"))?
			.props
			.aria()
			.pressed,
		Some(true.into())
	);

	tester.tap(&Finder::aria_role(Role::Button, "pl"))?;
	assert_eq!(app.l10n().locale(), "pl");
	tester.find(&Finder::text("Pocztówka"))?;
	tester.find(&Finder::text("Pozdrowienia: Lisbon!"))?;
	tester.find(&Finder::text("3 dni poza domem"))?;
	tester.find(&Finder::text("Jak liczy się 3"))?;
	assert!(tester.find(&Finder::text("Postcard")).is_err());
	Ok(())
}

#[test]
fn the_days_slider_rewrites_the_card_and_moves_the_gauge() -> Result<(), TestError> {
	let (app, mut tester) = mount("en")?;
	tester.set_slider_value(&Finder::label("Days away"), 22.0)?;
	assert_eq!(app.trip().days.get(), 22);
	tester.find(&Finder::text("22 days away from home"))?;
	tester.find(&Finder::text("How 22 counts"))?;

	// At 22 Polish says few and Arabic many, while English, Japanese, and
	// Portuguese stay other: one row per language, one chip per row.
	for tag in ["en", "ar", "ja", "pl", "pt-BR"] {
		tester.find(&Finder::aria_role(Role::Cell, tag))?;
	}
	assert_eq!(
		(
			chips(&tester, "few"),
			chips(&tester, "many"),
			chips(&tester, "other")
		),
		(1, 1, 3)
	);

	// At 1, English, Arabic, Polish, and Portuguese say one; Japanese never
	// does.
	tester.set_slider_value(&Finder::label("Days away"), 1.0)?;
	assert_eq!((chips(&tester, "one"), chips(&tester, "other")), (4, 1));
	Ok(())
}

/// How many gauge chips name `category`.
fn chips(tester: &WidgetTester, category: &str) -> usize {
	tester.find_all(&Finder::text(category)).len()
}

#[test]
fn tapping_the_weather_and_the_signature_cycles_their_selects() -> Result<(), TestError> {
	let (app, mut tester) = mount("en")?;
	tester.tap(&Finder::label("The sun has not stopped shining."))?;
	tester.find(&Finder::text("It has rained every afternoon."))?;
	tester.tap(&Finder::label("It has rained every afternoon."))?;
	tester.find(&Finder::text("The weather keeps changing its mind."))?;

	tester.tap(&Finder::label("They send their love"))?;
	tester.find(&Finder::text("She sends her love"))?;

	app.trip().letters.set(1);
	tester.pump();
	tester.find(&Finder::text("You have 1 letter waiting at home"))?;
	Ok(())
}

#[test]
fn arabic_lays_the_whole_screen_out_right_to_left() -> Result<(), TestError> {
	let (app, mut tester) = mount("ar")?;
	let direction = |tester: &WidgetTester| {
		let surface = tester.surface();
		let root = surface.root().expect("a committed root");
		surface.props(root).direction
	};
	assert_eq!(direction(&tester), Some(Direction::Rtl));
	tester.find(&Finder::text("بطاقة بريدية"))?;

	// The title sits at the right of a right-to-left screen.
	let title = tester.find(&Finder::text("بطاقة بريدية"))?;
	assert!(title.frame.x > VIEWPORT.0 / 2.0);

	app.l10n().set_locale(&locale("en"));
	tester.pump();
	assert_eq!(direction(&tester), Some(Direction::Ltr));
	let title = tester.find(&Finder::text("Postcard"))?;
	assert!(title.frame.x < VIEWPORT.0 / 2.0);
	Ok(())
}
