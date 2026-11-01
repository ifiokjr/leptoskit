//! Policies that count linked rows, `cx.count(…)`, against both databases:
//! team membership decides who sees and changes a project, a count caps how
//! many projects a team has, and property tests compare what each request
//! sees and may change with a plain Rust model of the same rules.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;

/// The most projects a team may have.
const PROJECT_LIMIT: i64 = 3;

/// The signed-in person: the global the policies read.
struct Viewer(Id<Person>);

/// Someone who can join teams. No policies.
#[table(name = "people")]
pub struct Person {
	pub id: Id<Self>,
	pub name: String,
}

/// A team. No policies.
#[table(name = "teams")]
pub struct Team {
	pub id: Id<Self>,
	pub name: String,
}

/// A person's place in a team. Members see their teams' memberships, and only
/// a team's admins change them: both rules count the table itself.
#[table(name = "members")]
#[policy(select, |member, cx| cx.global(|viewer: &Viewer| cx.exists(Member::query()
	.filter(|other| other.team_id.eq(member.team_id) & other.person_id.eq(viewer.0)))))]
#[policy(insert, update, delete, |member, cx| cx.global(|viewer: &Viewer| cx.exists(Member::query()
	.filter(|other| other.team_id.eq(member.team_id) & other.person_id.eq(viewer.0) & other.admin.eq(true)))))]
pub struct Member {
	pub id: Id<Self>,
	pub team_id: Id<Team>,
	pub person_id: Id<Person>,
	pub admin: bool,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A team's project: public ones are for everyone, the rest for the team's
/// members. Members change them, only admins delete them, and a team has at
/// most [`PROJECT_LIMIT`].
#[table(name = "projects")]
#[policy(select, |project, cx| project.public.eq(true) | in_team(project, cx))]
#[policy(insert, update, delete, |project, cx| in_team(project, cx))]
#[policy(deny insert, |project, cx| cx.count(Project::query()
	.filter(|other| other.team_id.eq(project.team_id))).gt(PROJECT_LIMIT),
	message = "The team has as many projects as it may")]
#[policy(deny delete, |project, cx| cx.global(|viewer: &Viewer| cx.count(Member::query()
	.filter(|member| member.team_id.eq(project.team_id) & member.person_id.eq(viewer.0) & member.admin.eq(true))).eq(0)),
	message = "Only a team's admins delete its projects")]
pub struct Project {
	pub id: Id<Self>,
	pub team_id: Id<Team>,
	pub name: String,
	pub public: bool,
}

/// Whether the viewer is a member of the team that owns `project`.
fn in_team(project: &ProjectColumns, cx: &Context) -> leptoskit_db::Expr {
	cx.global(|viewer: &Viewer| {
		cx.count(
			Member::query().filter(|member| {
				member.team_id.eq(project.team_id) & member.person_id.eq(viewer.0)
			}),
		)
		.gt(0)
	})
}

/// Two tables whose policies each count the other: a cycle that ends where
/// a table's read rule meets itself again.
#[table(name = "lefts")]
#[policy(select, |_, cx| cx.exists(Right::query()))]
pub struct Left {
	pub id: Id<Self>,
}

/// The other half of the cycle.
#[table(name = "rights")]
#[policy(select, |_, cx| cx.exists(Left::query()))]
pub struct Right {
	pub id: Id<Self>,
}

async fn create_schema(db: &Db) {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.statements.join(";\n"))
		.await
		.unwrap();
}

async fn on_both<F, Fut>(check: F)
where
	F: Fn(Db) -> Fut,
	Fut: Future<Output = ()>,
{
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	check(db).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		check(db).await;
	}
}

/// A world to check, by position: teams and people are numbered, and every
/// membership and project names them by number.
#[derive(Clone, Debug)]
struct Spec {
	teams: usize,
	people: usize,
	/// `(team, person, admin, deleted)`.
	members: Vec<(usize, usize, bool, bool)>,
	/// `(team, public)`.
	projects: Vec<(usize, bool)>,
	/// Who asks, or nobody.
	viewer: Option<usize>,
	/// The team the viewer adds a project to.
	insert_team: usize,
}

fn spec() -> impl Strategy<Value = Spec> {
	(1..=3_usize, 1..=4_usize)
		.prop_flat_map(|(teams, people)| {
			(
				Just(teams),
				Just(people),
				proptest::collection::vec(
					(
						0..teams,
						0..people,
						any::<bool>(),
						proptest::bool::weighted(0.25),
					),
					0..8,
				),
				proptest::collection::vec((0..teams, any::<bool>()), 0..9),
				proptest::option::of(0..people),
				0..teams,
			)
		})
		.prop_map(|(teams, people, members, projects, viewer, insert_team)| {
			Spec {
				teams,
				people,
				members,
				projects,
				viewer,
				insert_team,
			}
		})
}

/// The rules, in plain Rust.
impl Spec {
	fn member(&self, team: usize, person: Option<usize>) -> bool {
		self.members
			.iter()
			.any(|&(t, p, _, deleted)| t == team && Some(p) == person && !deleted)
	}

	fn admin(&self, team: usize, person: Option<usize>) -> bool {
		self.members
			.iter()
			.any(|&(t, p, admin, deleted)| t == team && Some(p) == person && admin && !deleted)
	}

	fn visible_projects(&self) -> Vec<usize> {
		(0..self.projects.len())
			.filter(|&index| {
				let (team, public) = self.projects[index];
				public || self.member(team, self.viewer)
			})
			.collect()
	}

	fn visible_members(&self) -> Vec<usize> {
		(0..self.members.len())
			.filter(|&index| {
				let (team, _, _, deleted) = self.members[index];
				!deleted && self.member(team, self.viewer)
			})
			.collect()
	}

	/// Whether the cap refuses a new private project in `team`. The count
	/// sees what the viewer may see: every project of a team they're in,
	/// the new one included, and otherwise only the public ones.
	fn over_the_limit(&self, team: usize) -> bool {
		let counted = if self.member(team, self.viewer) {
			self.projects.iter().filter(|(t, _)| *t == team).count() + 1
		} else {
			self.projects
				.iter()
				.filter(|(t, public)| *t == team && *public)
				.count()
		};
		counted > PROJECT_LIMIT as usize
	}

	fn may_insert(&self, team: usize) -> bool {
		self.member(team, self.viewer) && !self.over_the_limit(team)
	}

	fn changeable_projects(&self) -> usize {
		self.projects
			.iter()
			.filter(|(team, _)| self.member(*team, self.viewer))
			.count()
	}

	fn deletable_projects(&self) -> usize {
		self.projects
			.iter()
			.filter(|(team, _)| self.admin(*team, self.viewer))
			.count()
	}
}

/// The rows a [`Spec`] made, by position.
struct Seeded {
	teams: Vec<Id<Team>>,
	people: Vec<Id<Person>>,
	members: Vec<Id<Member>>,
	projects: Vec<Id<Project>>,
}

async fn seed(db: &Db, spec: &Spec) -> Seeded {
	let system = db.system("seeding the property test");
	Project::query().destroy(&system).await.unwrap();
	Member::query()
		.with_deleted()
		.destroy(&system)
		.await
		.unwrap();
	Team::query().destroy(&system).await.unwrap();
	Person::query().destroy(&system).await.unwrap();

	let mut seeded = Seeded {
		teams: Vec::new(),
		people: Vec::new(),
		members: Vec::new(),
		projects: Vec::new(),
	};

	for index in 0..spec.teams {
		let team = NewTeam {
			name: format!("team {index}"),
		};
		seeded.teams.push(team.insert(&system).await.unwrap().id);
	}

	for index in 0..spec.people {
		let person = NewPerson {
			name: format!("person {index}"),
		};
		seeded.people.push(person.insert(&system).await.unwrap().id);
	}

	for &(team, person, admin, deleted) in &spec.members {
		let mut member = NewMember {
			team_id: seeded.teams[team],
			person_id: seeded.people[person],
			admin,
		}
		.insert(&system)
		.await
		.unwrap();

		if deleted {
			member.delete(&system).await.unwrap();
		}

		seeded.members.push(member.id);
	}

	for (index, &(team, public)) in spec.projects.iter().enumerate() {
		let project = NewProject {
			team_id: seeded.teams[team],
			name: format!("project {index}"),
			public,
		};
		seeded
			.projects
			.push(project.insert(&system).await.unwrap().id);
	}

	seeded
}

fn sorted<T: Ord>(mut items: Vec<T>) -> Vec<T> {
	items.sort();
	items
}

/// Checks every action against the model for one world.
async fn agrees_with_the_model(db: &Db, mut spec: Spec) -> Result<(), TestCaseError> {
	let seeded = seed(db, &spec).await;
	let request = match spec.viewer {
		Some(person) => db.with_context(Context::new().with(Viewer(seeded.people[person]))),
		None => db.clone(),
	};

	let projects = Project::query().all(&request).await.unwrap();
	let expected = spec
		.visible_projects()
		.into_iter()
		.map(|index| seeded.projects[index]);
	prop_assert_eq!(
		sorted(projects.iter().map(|project| project.id).collect()),
		sorted(expected.collect::<Vec<_>>()),
		"visible projects"
	);
	prop_assert_eq!(
		Project::query().count(&request).await.unwrap(),
		spec.visible_projects().len() as u64
	);

	let members = Member::query().all(&request).await.unwrap();
	let expected = spec
		.visible_members()
		.into_iter()
		.map(|index| seeded.members[index]);
	prop_assert_eq!(
		sorted(members.iter().map(|member| member.id).collect()),
		sorted(expected.collect::<Vec<_>>()),
		"visible members"
	);

	let team = spec.insert_team;
	let inserted = NewProject {
		team_id: seeded.teams[team],
		name: "new".into(),
		public: false,
	}
	.insert(&request)
	.await;

	match inserted {
		Ok(_) => {
			prop_assert!(spec.may_insert(team), "the insert should have been refused");
			spec.projects.push((team, false));
		}
		Err(error) => {
			prop_assert!(error.is_forbidden(), "{error:?}");
			prop_assert!(
				!spec.may_insert(team),
				"the insert should have been allowed"
			);
			prop_assert_eq!(
				error.to_string() == "The team has as many projects as it may",
				spec.over_the_limit(team),
				"the refusal names the cap exactly when the cap matched"
			);
		}
	}

	let renamed = Project::update()
		.set(|project| project.name, "renamed")
		.run(&request)
		.await
		.unwrap();
	prop_assert_eq!(renamed, spec.changeable_projects() as u64, "renamed");

	let destroyed = Project::query().destroy(&request).await.unwrap();
	prop_assert_eq!(destroyed, spec.deletable_projects() as u64, "destroyed");
	Ok(())
}

fn run_property(db: &Db, runtime: &tokio::runtime::Runtime, cases: u32) {
	let mut runner = TestRunner::new(Config {
		cases,
		failure_persistence: None,
		..Config::default()
	});
	let outcome = runner.run(&spec(), |spec| {
		runtime.block_on(agrees_with_the_model(db, spec))
	});

	if let Err(error) = outcome {
		panic!("{error}");
	}
}

#[test]
fn policies_with_counts_agree_with_the_model_on_sqlite() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect("sqlite::memory:").await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 96);
}

#[test]
fn policies_with_counts_agree_with_the_model_on_postgres() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 48);
}

#[tokio::test(flavor = "multi_thread")]
async fn members_of_the_owning_team_see_and_change_its_projects() {
	on_both(async |db| {
		let spec = Spec {
			teams: 2,
			people: 3,
			members: vec![
				(0, 0, true, false),
				(0, 1, false, false),
				(1, 2, true, true),
			],
			projects: vec![(0, false), (1, false), (1, true)],
			viewer: None,
			insert_team: 0,
		};
		let seeded = seed(&db, &spec).await;
		let as_person =
			|person: usize| db.with_context(Context::new().with(Viewer(seeded.people[person])));

		// Ada and Grace are in team 0; Linus left team 1.
		let ada = as_person(0);
		let grace = as_person(1);
		let linus = as_person(2);
		assert_eq!(Project::query().count(&ada).await.unwrap(), 2);
		assert_eq!(Project::query().count(&grace).await.unwrap(), 2);
		assert_eq!(Project::query().count(&linus).await.unwrap(), 1);
		assert_eq!(Project::query().count(&db).await.unwrap(), 1);
		assert_eq!(Member::query().count(&grace).await.unwrap(), 2);
		assert_eq!(Member::query().count(&linus).await.unwrap(), 0);

		// Grace may change the team's project but not delete it.
		let mut project = Project::get(&grace, seeded.projects[0]).await.unwrap();
		project.name = "renamed".into();
		project.save(&grace).await.unwrap();
		let refused = project.destroy(&grace).await.unwrap_err();
		assert_eq!(
			refused.to_string(),
			"Only a team's admins delete its projects"
		);
		let project = Project::get(&ada, seeded.projects[0]).await.unwrap();
		project.destroy(&ada).await.unwrap();

		// Ada sees team 1's public project but may not change it, and may
		// not move one of her team's projects there either.
		let mut theirs = Project::get(&ada, seeded.projects[2]).await.unwrap();
		assert!(theirs.save(&ada).await.unwrap_err().is_forbidden());
		let mut ours = NewProject {
			team_id: seeded.teams[0],
			name: "ours".into(),
			public: false,
		}
		.insert(&ada)
		.await
		.unwrap();
		ours.team_id = seeded.teams[1];
		assert!(ours.save(&ada).await.unwrap_err().is_forbidden());
		assert_eq!(
			Project::get(&ada, ours.id).await.unwrap().team_id,
			seeded.teams[0]
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn counts_cap_how_many_rows_there_are() {
	on_both(async |db| {
		let spec = Spec {
			teams: 1,
			people: 1,
			members: vec![(0, 0, false, false)],
			projects: vec![(0, true); PROJECT_LIMIT as usize - 1],
			viewer: None,
			insert_team: 0,
		};
		let seeded = seed(&db, &spec).await;
		let ada = db.with_context(Context::new().with(Viewer(seeded.people[0])));
		let new = || {
			NewProject {
				team_id: seeded.teams[0],
				name: "one more".into(),
				public: false,
			}
		};

		new().insert(&ada).await.unwrap();
		let refused = new().insert(&ada).await.unwrap_err();
		assert_eq!(
			refused.to_string(),
			"The team has as many projects as it may"
		);
		assert_eq!(
			Project::query().count(&ada).await.unwrap(),
			PROJECT_LIMIT as u64
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn policies_that_count_each_other_still_finish() {
	on_both(async |db| {
		let system = db.system("seeding the cycle");
		let request = db.with_context(Context::new());
		NewLeft {}.insert(&system).await.unwrap();
		assert_eq!(Left::query().count(&request).await.unwrap(), 0);

		// Each side's rule sees the other side, whose own rule counts the
		// first side's rows as they are.
		let right = NewRight {}.insert(&system).await.unwrap();
		assert_eq!(Left::query().count(&request).await.unwrap(), 1);
		assert_eq!(Right::query().count(&request).await.unwrap(), 1);

		right.destroy(&system).await.unwrap();
		assert_eq!(Left::query().count(&request).await.unwrap(), 0);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn columns_are_compared_but_never_written() {
	on_both(async |db| {
		let set = Project::update()
			.set(|project| project.name, Team::columns().name)
			.run(&db.system("a misuse"))
			.await
			.unwrap_err();
		assert!(
			format!("{set:?}").contains("has no value of its own"),
			"{set:?}"
		);

		let listed = Project::query()
			.filter(|project| project.name.in_list([Team::columns().name]))
			.count(&db.system("a misuse"))
			.await
			.unwrap_err();
		assert!(format!("{listed:?}").contains("has no value of its own"));

		// Outside a count, another table's column names no row.
		let unrelated = Project::query()
			.filter(|project| project.name.eq(Team::columns().name))
			.count(&db.system("a misuse"))
			.await
			.unwrap_err();
		assert!(
			format!("{unrelated:?}")
				.contains("belongs to neither the row being checked nor a table it counts"),
			"{unrelated:?}"
		);

		// The table's own columns compare with each other.
		let system = db.system("seeding");
		let team = NewTeam {
			name: "same".into(),
		}
		.insert(&system)
		.await
		.unwrap();
		let other = NewTeam {
			name: "different".into(),
		}
		.insert(&system)
		.await
		.unwrap();
		assert_eq!(
			Team::query()
				.filter(|team| team.name.eq(Team::columns().name))
				.count(&system)
				.await
				.unwrap(),
			2
		);
		assert_ne!(team.id, other.id);
	})
	.await;
}
