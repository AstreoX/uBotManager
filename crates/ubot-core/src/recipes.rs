//! Recipes: turning a recipe into changes, and saving the enabled set as a user recipe.

use crate::model::{Change, ChangeKind, LocalizedText, Recipe};
use crate::workspace::Workspace;

/// Changes that make `packages` (or, when that is empty, the recipe's packages) installed and
/// enabled: install what is missing, enable what is disabled; nothing else.
pub fn recipe_changes(
    workspace: &Workspace,
    recipe_id: Option<&str>,
    packages: &[String],
) -> Vec<Change> {
    let names: Vec<String> = if !packages.is_empty() {
        packages.to_vec()
    } else {
        recipe_id
            .and_then(|id| {
                workspace
                    .recipes()
                    .into_iter()
                    .find(|recipe| recipe.id.eq_ignore_ascii_case(id))
            })
            .map(|recipe| recipe.packages)
            .unwrap_or_default()
    };

    let mut changes: Vec<Change> = Vec::new();
    for name in names {
        if changes
            .iter()
            .any(|change| change.package.eq_ignore_ascii_case(name.trim()))
        {
            continue;
        }
        match workspace.installed(&name) {
            Some(plugin) if !workspace.is_enabled(plugin) => {
                changes.push(Change::new(
                    ChangeKind::Enable,
                    workspace.display_name(&name),
                    None,
                ));
            }
            Some(_) => {}
            None if workspace.index.is_available(&name) => {
                changes.push(Change::new(
                    ChangeKind::Install,
                    workspace.display_name(&name),
                    None,
                ));
            }
            None => {}
        }
    }
    changes
}

/// "My ROS setup" → "user-my-ros-setup".
pub fn user_recipe_id(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "user-recipe".into()
    } else {
        format!("user-{slug}")
    }
}

/// A user recipe of the currently enabled uBot packages.
pub fn user_recipe(workspace: &Workspace, name: &str) -> Recipe {
    let mut packages: Vec<String> = workspace
        .ubot_plugins()
        .into_iter()
        .filter(|plugin| workspace.is_enabled(plugin))
        .map(|plugin| workspace.display_name(&plugin.name))
        .collect();
    packages.sort_by_key(|package| package.to_lowercase());
    Recipe {
        id: user_recipe_id(name),
        name: LocalizedText::en(name.trim()),
        description: LocalizedText::en(""),
        packages,
        checks: Vec::new(),
        available: true,
        user: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::tests::sample_workspace;

    #[test]
    fn changes_for_recipes() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = sample_workspace(dir.path(), false);
        let changes = recipe_changes(&workspace, Some("ros"), &[]);
        assert_eq!(
            changes,
            [
                Change::new(ChangeKind::Enable, "UBotSensor", None),
                Change::new(ChangeKind::Install, "UBotROS", None)
            ]
        );
        let changes = recipe_changes(
            &workspace,
            Some("ignored"),
            &[
                "ubotcore".into(),
                "UBotVehicles".into(),
                "Nope".into(),
                "ubotros".into(),
            ],
        );
        assert_eq!(changes, [Change::new(ChangeKind::Install, "UBotROS", None)]);
        assert!(recipe_changes(&workspace, Some("missing"), &[]).is_empty());

        let recipe = user_recipe(&workspace, "  My ROS setup! ");
        assert_eq!(recipe.id, "user-my-ros-setup");
        assert_eq!(recipe.name.en, "My ROS setup!");
        assert_eq!(recipe.packages, ["UBotCore", "UBotEnvironment"]);
        assert_eq!(user_recipe_id("!!"), "user-recipe");
    }
}
