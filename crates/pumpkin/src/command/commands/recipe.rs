use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::string::StringArgumentType;
use crate::command::argument_types::entity::EntityArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::command::suggestion::provider::SuggestionProvider;
use crate::command::suggestion::suggestions::{Suggestions, SuggestionsBuilder};
use crate::entity::EntityBase;
use pumpkin_data::translation;
use pumpkin_protocol::java::client::play::CRecipeBookAdd;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

const DESCRIPTION: &str = "Gives or takes player recipes.";
const PERMISSION: &str = "minecraft:command.recipe";

static ERROR_RECIPE_NOT_FOUND: CommandErrorType<1> =
    CommandErrorType::new(translation::java::RECIPE_NOTFOUND, "Unknown recipe: %s");

struct RecipeSuggestionProvider;

impl SuggestionProvider for RecipeSuggestionProvider {
    fn suggest(
        &self,
        context: &CommandContext,
        mut builder: SuggestionsBuilder,
    ) -> Pin<Box<dyn Future<Output = Suggestions> + Send>> {
        let server = context.source.server.clone();

        Box::pin(async move {
            builder = builder.suggest("*");
            if let Some(server) = server {
                let mut ids: Vec<_> = server
                    .recipe_manager
                    .valid_recipe_ids()
                    .await
                    .into_iter()
                    .collect();
                ids.sort_unstable();
                for id in ids {
                    builder = builder.suggest(id);
                }
            }
            builder.build()
        })
    }
}

struct RecipeGiveExecutor;

impl CommandExecutor for RecipeGiveExecutor {
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            let targets = EntityArgumentType::get_players(context, "targets").await?;
            let recipe_str = StringArgumentType::get(context, "recipe")?;

            let server = context.source.server.as_ref().ok_or_else(|| {
                ERROR_RECIPE_NOT_FOUND
                    .create_without_context(TextComponent::text(recipe_str.to_string()))
            })?;

            let is_all = recipe_str == "*";
            let all_dynamic = server.recipe_manager.get_dynamic_recipes_internal().await;
            let builtins = crate::server::recipe::RecipeManager::built_in_recipe_ids();
            let valid = server.recipe_manager.valid_recipe_ids().await;
            let requested_id = (!is_all)
                .then(|| crate::server::recipe::canonicalize_recipe_id(recipe_str))
                .filter(|id| valid.contains(id));
            if !is_all && requested_id.is_none() {
                return Err(ERROR_RECIPE_NOT_FOUND
                    .create_without_context(TextComponent::text(recipe_str.to_string())));
            }

            let matching_builtin_ids: HashSet<String> = if is_all {
                builtins.clone()
            } else if builtins.contains(requested_id.as_ref().expect("validated recipe id")) {
                [requested_id.clone().expect("validated recipe id")]
                    .into_iter()
                    .collect()
            } else {
                HashSet::new()
            };
            let matching_recipes: Vec<_> = if is_all {
                all_dynamic.clone()
            } else {
                all_dynamic
                    .iter()
                    .filter(|recipe| {
                        crate::server::recipe::recipe_id(recipe)
                            == requested_id.as_deref().expect("validated recipe id")
                    })
                    .cloned()
                    .collect()
            };
            let recipe_count = if is_all { valid.len() } else { 1 };

            for player in &targets {
                if is_all {
                    for id in &valid {
                        player.unlock_recipe(id).await;
                    }
                } else if let Some(id) = requested_id.as_ref() {
                    player.unlock_recipe(id).await;
                }
                if let crate::net::ClientPlatform::Java(java_client) = player.client.as_ref() {
                    // `give` is an additive packet.  Do not resend every
                    // generated recipe; send exactly the requested displays.
                    java_client
                        .send_packet_now(&CRecipeBookAdd::new_filtered(
                            false,
                            &matching_recipes,
                            &matching_builtin_ids,
                        ))
                        .await;
                }
            }

            let recipe_count_str = recipe_count.to_string();
            if targets.len() == 1 {
                let msg = TextComponent::translate_cross(
                    translation::java::COMMANDS_RECIPE_GIVE_SUCCESS_SINGLE,
                    translation::java::COMMANDS_RECIPE_GIVE_SUCCESS_SINGLE,
                    [
                        TextComponent::text(recipe_count_str),
                        targets[0].get_display_name().await,
                    ],
                );
                context.source.send_feedback(msg, true).await;
            } else {
                let msg = TextComponent::translate_cross(
                    translation::java::COMMANDS_RECIPE_GIVE_SUCCESS_MULTIPLE,
                    translation::java::COMMANDS_RECIPE_GIVE_SUCCESS_MULTIPLE,
                    [
                        TextComponent::text(recipe_count_str),
                        TextComponent::text(targets.len().to_string()),
                    ],
                );
                context.source.send_feedback(msg, true).await;
            }

            Ok((targets.len() * recipe_count) as i32)
        })
    }
}

struct RecipeTakeExecutor;

impl CommandExecutor for RecipeTakeExecutor {
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            let targets = EntityArgumentType::get_players(context, "targets").await?;
            let recipe_str = StringArgumentType::get(context, "recipe")?;

            let server = context.source.server.as_ref().ok_or_else(|| {
                ERROR_RECIPE_NOT_FOUND
                    .create_without_context(TextComponent::text(recipe_str.to_string()))
            })?;

            let is_all = recipe_str == "*";
            let all_recipes = server.recipe_manager.get_dynamic_recipes_internal().await;
            let all_builtin_ids = crate::server::recipe::RecipeManager::built_in_recipe_ids();
            let valid = server.recipe_manager.valid_recipe_ids().await;
            let requested_id = (!is_all)
                .then(|| crate::server::recipe::canonicalize_recipe_id(recipe_str))
                .filter(|id| valid.contains(id));
            if !is_all && requested_id.is_none() {
                return Err(ERROR_RECIPE_NOT_FOUND
                    .create_without_context(TextComponent::text(recipe_str.to_string())));
            }

            let removed_ids: HashSet<String> = if is_all {
                valid.clone()
            } else {
                [requested_id.clone().expect("validated recipe id")]
                    .into_iter()
                    .collect()
            };
            let taken_count = removed_ids.len();

            for player in &targets {
                for id in &removed_ids {
                    player.revoke_recipe(id).await;
                }
                if let crate::net::ClientPlatform::Java(java_client) = player.client.as_ref() {
                    let known_recipes = player.known_recipes().await;
                    let recipes_to_keep = all_recipes
                        .iter()
                        .filter(|recipe| {
                            known_recipes.contains(&crate::server::recipe::recipe_id(recipe))
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    let mut builtin_to_keep: HashSet<String> =
                        all_builtin_ids.difference(&removed_ids).cloned().collect();
                    builtin_to_keep.retain(|id| known_recipes.contains(id));
                    java_client
                        .send_packet_now(&CRecipeBookAdd::new_filtered(
                            true,
                            &recipes_to_keep,
                            &builtin_to_keep,
                        ))
                        .await;
                }
            }

            let taken_count_str = taken_count.to_string();
            if targets.len() == 1 {
                let msg = TextComponent::translate_cross(
                    translation::java::COMMANDS_RECIPE_TAKE_SUCCESS_SINGLE,
                    translation::java::COMMANDS_RECIPE_TAKE_SUCCESS_SINGLE,
                    [
                        TextComponent::text(taken_count_str),
                        targets[0].get_display_name().await,
                    ],
                );
                context.source.send_feedback(msg, true).await;
            } else {
                let msg = TextComponent::translate_cross(
                    translation::java::COMMANDS_RECIPE_TAKE_SUCCESS_MULTIPLE,
                    translation::java::COMMANDS_RECIPE_TAKE_SUCCESS_MULTIPLE,
                    [
                        TextComponent::text(taken_count_str),
                        TextComponent::text(targets.len().to_string()),
                    ],
                );
                context.source.send_feedback(msg, true).await;
            }

            Ok((targets.len() * taken_count) as i32)
        })
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &mut PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    let builder = command("recipe", DESCRIPTION)
        .requires(PERMISSION)
        .then(
            literal("give").then(
                argument("targets", EntityArgumentType::Players).then(
                    argument("recipe", StringArgumentType::SingleWord)
                        .suggests(RecipeSuggestionProvider)
                        .executes(RecipeGiveExecutor),
                ),
            ),
        )
        .then(
            literal("take").then(
                argument("targets", EntityArgumentType::Players).then(
                    argument("recipe", StringArgumentType::SingleWord)
                        .suggests(RecipeSuggestionProvider)
                        .executes(RecipeTakeExecutor),
                ),
            ),
        );

    dispatcher.register(builder);
}
