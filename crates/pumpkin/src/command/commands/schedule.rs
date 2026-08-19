use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::string::StringArgumentType;
use crate::command::argument_types::time::TimeArgumentType;
use crate::command::commands::function::canonical_function_id;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_util::PermissionLvl;
use pumpkin_util::identifier::Identifier;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

const DESCRIPTION: &str = "Schedules a datapack function or function tag.";
const PERMISSION: &str = "minecraft:command.schedule";
const SAME_TICK: CommandErrorType<0> =
    CommandErrorType::new("commands.schedule.same_tick", "commands.schedule.same_tick");
const UNKNOWN_TARGET: CommandErrorType<1> = CommandErrorType::new(
    "commands.schedule.unknown_function",
    "commands.schedule.unknown_function",
);
const CLEAR_FAILED: CommandErrorType<1> = CommandErrorType::new(
    "commands.schedule.cleared.failure",
    "commands.schedule.cleared.failure",
);

fn canonical_target(raw: &str) -> Option<(String, bool)> {
    if let Some(tag) = raw.strip_prefix('#') {
        let identifier = Identifier::parse(tag).ok()?;
        let (namespace, path) = identifier.view();
        let path = format!("function/{path}");
        let identifier = Identifier::new(namespace.to_owned(), path).ok()?;
        Some((identifier.to_string(), true))
    } else {
        canonical_function_id(raw).map(|id| (id, false))
    }
}

struct ScheduleExecutor {
    replace: bool,
}

impl CommandExecutor for ScheduleExecutor {
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            let raw = StringArgumentType::get(context, "function")?;
            let (id, tag) = canonical_target(raw).ok_or_else(|| {
                UNKNOWN_TARGET
                    .create_without_context_args_slice(&[TextComponent::text(raw.to_owned())])
            })?;
            let exists = if tag {
                context
                    .server()
                    .recipe_manager
                    .tag_snapshot()
                    .await
                    .tags
                    .contains_key(&id)
            } else {
                context
                    .server()
                    .recipe_manager
                    .function(&id)
                    .await
                    .is_some()
            };
            if !exists {
                return Err(UNKNOWN_TARGET
                    .create_without_context_args_slice(&[TextComponent::text(raw.to_owned())]));
            }
            let delay = TimeArgumentType::get(context, "time")?;
            if delay == 0 {
                return Err(SAME_TICK.create_without_context());
            }
            let now = context.world().get_world_age().await;
            let trigger_time = now.saturating_add(i64::from(delay));
            context
                .server()
                .schedule_function(id.clone(), trigger_time, tag, self.replace)
                .await;
            context
                .source
                .send_feedback(
                    TextComponent::text(format!(
                        "Scheduled {}{} at game time {trigger_time}",
                        if tag { "#" } else { "" },
                        id
                    )),
                    true,
                )
                .await;
            Ok(trigger_time.rem_euclid(i64::from(i32::MAX)) as i32)
        })
    }
}

struct ClearExecutor;

impl CommandExecutor for ClearExecutor {
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            let raw = StringArgumentType::get(context, "function")?;
            let (id, tag) = canonical_target(raw).ok_or_else(|| {
                CLEAR_FAILED
                    .create_without_context_args_slice(&[TextComponent::text(raw.to_owned())])
            })?;
            let removed = context
                .server()
                .clear_scheduled_functions(&id, Some(tag))
                .await;
            if removed == 0 {
                return Err(CLEAR_FAILED
                    .create_without_context_args_slice(&[TextComponent::text(raw.to_owned())]));
            }
            context
                .source
                .send_feedback(
                    TextComponent::text(format!("Cleared {removed} scheduled event(s) for {raw}")),
                    true,
                )
                .await;
            Ok(removed as i32)
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
        command("schedule", DESCRIPTION)
            .requires(PERMISSION)
            .then(
                literal("function").then(
                    argument("function", StringArgumentType::SingleWord).then(
                        argument("time", TimeArgumentType::new(1))
                            .executes(ScheduleExecutor { replace: true })
                            .then(literal("append").executes(ScheduleExecutor { replace: false }))
                            .then(literal("replace").executes(ScheduleExecutor { replace: true })),
                    ),
                ),
            )
            .then(literal("clear").then(
                argument("function", StringArgumentType::GreedyPhrase).executes(ClearExecutor),
            )),
    );
}

#[cfg(test)]
mod tests {
    use super::canonical_target;

    #[test]
    fn canonicalizes_function_and_function_tag_targets() {
        assert_eq!(
            canonical_target("foo:bar"),
            Some(("foo:bar".to_owned(), false))
        );
        assert_eq!(
            canonical_target("#foo:tick"),
            Some(("foo:function/tick".to_owned(), true))
        );
        assert!(canonical_target("#foo:").is_none());
    }
}
