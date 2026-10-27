//! The sky the observatory watches: a catalogue of bright stars and a year of
//! nightly observations, generated the same way on every run.

/// One catalogued star.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Star {
	/// The star's proper name.
	pub name: &'static str,
	/// The constellation it belongs to.
	pub constellation: &'static str,
	/// Apparent visual magnitude: smaller is brighter, and the brightest
	/// stars are negative.
	pub magnitude: f64,
	/// Distance from the Sun, in light years.
	pub distance: f64,
	/// Spectral class, from hot blue O to cool red M.
	pub class: &'static str,
}

const fn star(
	name: &'static str,
	constellation: &'static str,
	magnitude: f64,
	distance: f64,
	class: &'static str,
) -> Star {
	Star {
		name,
		constellation,
		magnitude,
		distance,
		class,
	}
}

/// The forty-four brightest stars in the night sky, brightest first.
pub const STARS: [Star; 44] = [
	star("Sirius", "Canis Major", -1.46, 8.6, "A1V"),
	star("Canopus", "Carina", -0.74, 310.0, "A9II"),
	star("Rigil Kentaurus", "Centaurus", -0.27, 4.4, "G2V"),
	star("Arcturus", "Boötes", -0.05, 37.0, "K1.5III"),
	star("Vega", "Lyra", 0.03, 25.0, "A0V"),
	star("Capella", "Auriga", 0.08, 43.0, "G8III"),
	star("Rigel", "Orion", 0.13, 860.0, "B8Ia"),
	star("Procyon", "Canis Minor", 0.34, 11.5, "F5IV"),
	star("Achernar", "Eridanus", 0.46, 139.0, "B6V"),
	star("Betelgeuse", "Orion", 0.50, 550.0, "M1Ia"),
	star("Hadar", "Centaurus", 0.61, 390.0, "B1III"),
	star("Altair", "Aquila", 0.76, 16.7, "A7V"),
	star("Acrux", "Crux", 0.76, 320.0, "B0.5IV"),
	star("Aldebaran", "Taurus", 0.86, 65.0, "K5III"),
	star("Antares", "Scorpius", 0.96, 550.0, "M1.5Iab"),
	star("Spica", "Virgo", 0.97, 250.0, "B1III"),
	star("Pollux", "Gemini", 1.14, 34.0, "K0III"),
	star("Fomalhaut", "Piscis Austrinus", 1.16, 25.0, "A3V"),
	star("Deneb", "Cygnus", 1.25, 2600.0, "A2Ia"),
	star("Mimosa", "Crux", 1.25, 280.0, "B0.5III"),
	star("Regulus", "Leo", 1.40, 79.0, "B8IV"),
	star("Adhara", "Canis Major", 1.50, 430.0, "B2II"),
	star("Castor", "Gemini", 1.58, 51.0, "A1V"),
	star("Shaula", "Scorpius", 1.62, 570.0, "B2IV"),
	star("Gacrux", "Crux", 1.64, 88.0, "M3.5III"),
	star("Bellatrix", "Orion", 1.64, 250.0, "B2III"),
	star("Elnath", "Taurus", 1.65, 130.0, "B7III"),
	star("Miaplacidus", "Carina", 1.67, 113.0, "A1III"),
	star("Alnilam", "Orion", 1.69, 2000.0, "B0Ia"),
	star("Alnair", "Grus", 1.74, 101.0, "B6V"),
	star("Alnitak", "Orion", 1.77, 1260.0, "O9.5Iab"),
	star("Alioth", "Ursa Major", 1.77, 83.0, "A1III"),
	star("Dubhe", "Ursa Major", 1.79, 123.0, "K0III"),
	star("Mirfak", "Perseus", 1.80, 510.0, "F5Ib"),
	star("Wezen", "Canis Major", 1.83, 1600.0, "F8Ia"),
	star("Kaus Australis", "Sagittarius", 1.85, 143.0, "B9.5III"),
	star("Sargas", "Scorpius", 1.86, 300.0, "F1II"),
	star("Avior", "Carina", 1.86, 630.0, "K3III"),
	star("Alkaid", "Ursa Major", 1.86, 104.0, "B3V"),
	star("Menkalinan", "Auriga", 1.90, 81.0, "A1IV"),
	star("Atria", "Triangulum Australe", 1.91, 391.0, "K2Ib"),
	star("Alhena", "Gemini", 1.92, 109.0, "A1IV"),
	star("Peacock", "Pavo", 1.94, 179.0, "B2IV"),
	star("Polaris", "Ursa Minor", 1.98, 433.0, "F7Ib"),
];

/// How many nights the log covers.
pub const NIGHTS: u16 = 365;

/// One star seen on one night.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
	/// The night, from 1 to [`NIGHTS`].
	pub night: u16,
	/// Minutes after 18:00 when the observation was made.
	pub minute: u16,
	/// The observed star, an index into [`STARS`].
	pub star: usize,
	/// How steady the air was, from 1 (boiling) to 5 (perfect).
	pub seeing: u8,
	/// What the observer wrote down, if anything.
	pub note: Option<&'static str>,
}

impl Observation {
	/// The observed star.
	pub fn star(&self) -> &'static Star {
		&STARS[self.star]
	}

	/// The local time of the observation, as `HH:MM`.
	pub fn time(&self) -> String {
		let minutes = 18 * 60 + u32::from(self.minute);
		format!("{:02}:{:02}", (minutes / 60) % 24, minutes % 60)
	}

	/// The seeing as five stars, filled to its grade.
	pub fn seeing_stars(&self) -> String {
		(1..=5)
			.map(|grade| if grade <= self.seeing { '★' } else { '☆' })
			.collect()
	}
}

const NOTES: [&str; 6] = [
	"Thin cirrus drifting in from the west.",
	"A meteor crossed the field mid-exposure.",
	"Dew on the corrector plate; wiped and refocused.",
	"Noticeably brighter than the chart says.",
	"Satellite trail through the frame.",
	"Twinkling hard near the horizon.",
];

/// A year of observations, ordered by night and then by time: between 40
/// and 71 a night, from the same seed on every run.
pub fn observations() -> Vec<Observation> {
	let mut random = SplitMix64(0x0b5e_7a70_7e1e_5c0e);
	let mut log = Vec::with_capacity(usize::from(NIGHTS) * 56);

	for night in 1..=NIGHTS {
		let count = 40 + random.below(32);
		let mut minute = 0;
		for _ in 0..count {
			minute += 2 + random.below(10) as u16;
			log.push(Observation {
				night,
				minute,
				star: random.below(STARS.len() as u64) as usize,
				seeing: 1 + random.below(5) as u8,
				note: (random.below(7) == 0)
					.then(|| NOTES[random.below(NOTES.len() as u64) as usize]),
			});
		}
	}

	log
}

/// The moon's phase on a night, as the glyph an almanac prints: the log
/// starts at a new moon, and a lunation lasts 29.53 nights.
pub fn moon_phase(night: u16) -> char {
	const PHASES: [char; 8] = ['🌑', '🌒', '🌓', '🌔', '🌕', '🌖', '🌗', '🌘'];
	let age = (f64::from(night) % 29.53) / 29.53;
	PHASES[((age * 8.0).round() as usize) % PHASES.len()]
}

/// A small, fast, seedable generator, so the sky is the same on every run
/// without a dependency.
struct SplitMix64(u64);

impl SplitMix64 {
	fn next(&mut self) -> u64 {
		self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
		let mut value = self.0;
		value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
		value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
		value ^ (value >> 31)
	}

	/// A value in `0..bound`.
	fn below(&mut self, bound: u64) -> u64 {
		self.next() % bound
	}
}
