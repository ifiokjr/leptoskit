//! Running `git` about a project's own directory.

use std::path::Path;
use std::process::Command;

/// The variables that point git at a repository whatever directory it runs
/// in. Git sets some of them for hooks, so `leptoskit` run from one, such as
/// a pre-push hook's tests in a linked worktree, would otherwise read, or
/// change, the hook's repository instead of the project's.
const REPOSITORY_VARIABLES: &[&str] = &[
	"GIT_DIR",
	"GIT_WORK_TREE",
	"GIT_INDEX_FILE",
	"GIT_COMMON_DIR",
	"GIT_OBJECT_DIRECTORY",
	"GIT_ALTERNATE_OBJECT_DIRECTORIES",
	"GIT_NAMESPACE",
	"GIT_PREFIX",
];

/// `git -C <root>`, which finds its repository from `root` alone.
pub(crate) fn command(root: &Path) -> Command {
	let mut command = Command::new("git");
	command.arg("-C").arg(root);

	for variable in REPOSITORY_VARIABLES {
		command.env_remove(variable);
	}

	command
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_hook_s_repository_variables_are_left_behind() {
		let root = tempfile::tempdir().unwrap();
		let mut git = command(root.path());
		let removed: Vec<_> = git
			.get_envs()
			.filter(|(_, value)| value.is_none())
			.map(|(name, _)| name.to_string_lossy().into_owned())
			.collect();

		for variable in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
			assert!(removed.iter().any(|name| name == variable), "{variable}");
		}

		let output = git
			.args(["rev-parse", "--is-inside-work-tree"])
			.output()
			.unwrap();
		assert!(
			!output.status.success(),
			"a temporary directory isn't a repository"
		);
	}
}
