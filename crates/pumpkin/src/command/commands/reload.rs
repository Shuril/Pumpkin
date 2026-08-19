use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

use crate::command::argument_builder::{ArgumentBuilder, command};
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};

const DESCRIPTION: &str = "Reloads enabled datapack data.";
const PERMISSION: &str = "minecraft:command.reload";

const RELOAD_FAILED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_RELOAD_FAILURE,
    translation::bedrock::COMMANDS_RELOAD_ERROR,
);

struct ReloadExecutor;

impl CommandExecutor for ReloadExecutor {
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            context
                .source
                .send_feedback(
                    TextComponent::translate_cross(
                        translation::java::COMMANDS_RELOAD_SUCCESS,
                        translation::bedrock::COMMANDS_RELOAD_STARTED,
                        [],
                    ),
                    false,
                )
                .await;

            match context.server().reload_datapacks().await {
                Ok(_) => {
                    context
                        .source
                        .send_feedback(
                            TextComponent::translate_cross(
                                translation::java::COMMANDS_RELOAD_SUCCESS,
                                translation::bedrock::COMMANDS_RELOAD_SUCCESS,
                                [],
                            ),
                            true,
                        )
                        .await;
                    Ok(1)
                }
                Err(error) => {
                    tracing::error!(%error, "datapack reload failed");
                    Err(RELOAD_FAILED.create_without_context())
                }
            }
        })
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &mut PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("reload", DESCRIPTION)
            .requires(PERMISSION)
            .executes(ReloadExecutor),
    );
}
