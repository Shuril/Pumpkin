use crate::command::argument_builder::{ArgumentBuilder, argument, command};
use crate::command::argument_types::core::string::StringArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_util::PermissionLvl;
use pumpkin_util::identifier::Identifier;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

const DESCRIPTION: &str = "Executes a datapack function.";
const PERMISSION: &str = "minecraft:command.function";
const MAX_FUNCTION_DEPTH: u8 = 64;

static ERROR_UNKNOWN_FUNCTION: CommandErrorType<1> =
    CommandErrorType::new("commands.function.unknown", "Unknown function: %s");
static ERROR_RECURSION: CommandErrorType<0> = CommandErrorType::new(
    "commands.function.recursion",
    "Function call depth exceeded the server safety limit",
);

pub(crate) fn canonical_function_id(raw: &str) -> Option<String> {
    let id = if raw.contains(':') {
        raw.to_owned()
    } else {
        format!("minecraft:{raw}")
    };
    Identifier::parse(&id)
        .ok()
        .map(|identifier| identifier.to_string())
}

pub(crate) fn canonical_function_tag_id(raw: &str) -> Option<String> {
    let raw = raw.strip_prefix('#')?;
    let identifier = Identifier::parse(raw).ok()?;
    let (namespace, path) = identifier.view();
    Identifier::new(namespace.to_owned(), format!("function/{path}"))
        .ok()
        .map(|identifier| identifier.to_string())
}

struct FunctionExecutor;

impl CommandExecutor for FunctionExecutor {
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            let raw_id = StringArgumentType::get(context, "name")?;
            let (function_ids, display_id) = if raw_id.starts_with('#') {
                let tag_id = canonical_function_tag_id(raw_id).ok_or_else(|| {
                    ERROR_UNKNOWN_FUNCTION
                        .create_without_context(TextComponent::text(raw_id.to_owned()))
                })?;
                let ids = context
                    .server()
                    .recipe_manager
                    .tag_snapshot()
                    .await
                    .ordered_members(&tag_id);
                let mut present = Vec::new();
                for id in ids {
                    if context
                        .server()
                        .recipe_manager
                        .function(&id)
                        .await
                        .is_some()
                    {
                        present.push(id);
                    }
                }
                if present.is_empty() {
                    return Err(ERROR_UNKNOWN_FUNCTION
                        .create_without_context(TextComponent::text(raw_id.to_owned())));
                }
                (present, tag_id)
            } else {
                let id = canonical_function_id(raw_id).ok_or_else(|| {
                    ERROR_UNKNOWN_FUNCTION
                        .create_without_context(TextComponent::text(raw_id.to_owned()))
                })?;
                (vec![id.clone()], id)
            };
            let source = context.source.as_ref();
            if source.function_depth >= MAX_FUNCTION_DEPTH {
                return Err(ERROR_RECURSION.create_without_context());
            }
            let child_source = source
                .clone()
                .with_function_depth(source.function_depth.saturating_add(1));
            let mut executed = 0;
            for id in function_ids {
                let commands = context
                    .server()
                    .recipe_manager
                    .function(&id)
                    .await
                    .ok_or_else(|| {
                        ERROR_UNKNOWN_FUNCTION
                            .create_without_context(TextComponent::text(id.clone()))
                    })?;
                for command in commands {
                    // Vanilla continues after a failed command in a function.
                    // Keep the line-level error visible to the same source
                    // while letting later commands run and contribute to the
                    // return count.
                    // Acquire the dispatcher only for the individual command.  A
                    // function may invoke `/function` recursively; keeping the
                    // read guard across the whole function would prevent a
                    // queued dispatcher writer from completing and could make
                    // that recursive call wait forever on the same lock.
                    let command_result = {
                        let dispatcher = context.server().command_dispatcher.read().await;
                        dispatcher.execute_input(&command, &child_source).await
                    };
                    match command_result {
                        Ok(result) => executed += result,
                        Err(error) => {
                            CommandDispatcher::send_error_to_source(&child_source, error, &command)
                                .await;
                        }
                    }
                }
            }
            if executed > 0 {
                context
                    .source
                    .send_feedback(
                        TextComponent::text(format!("Function {display_id} returned {executed}")),
                        true,
                    )
                    .await;
            }
            Ok(executed)
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
        command("function", DESCRIPTION)
            .requires(PERMISSION)
            .then(argument("name", StringArgumentType::SingleWord).executes(FunctionExecutor)),
    );
}

#[cfg(test)]
mod tests {
    use super::{canonical_function_id, canonical_function_tag_id};

    #[test]
    fn function_ids_are_canonical_and_validated() {
        assert_eq!(
            canonical_function_id("demo:start").as_deref(),
            Some("demo:start")
        );
        assert_eq!(
            canonical_function_id("start").as_deref(),
            Some("minecraft:start")
        );
        assert!(canonical_function_id("demo:").is_none());
        assert!(canonical_function_id("demo:bad name").is_none());
        assert_eq!(
            canonical_function_tag_id("#demo:tick").as_deref(),
            Some("demo:function/tick")
        );
        assert!(canonical_function_tag_id("demo:tick").is_none());
    }
}
