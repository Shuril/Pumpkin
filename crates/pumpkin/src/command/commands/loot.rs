use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::coordinates::block_pos::BlockPosArgumentType;
use crate::command::argument_types::core::string::StringArgumentType;
use crate::command::argument_types::entity::EntityArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::world::loot::{LootTableExt, derive_loot_seed};
use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

const DESCRIPTION: &str =
    "Drops the given loot table into the specified inventory or into the world.";
const PERMISSION: &str = "minecraft:command.loot";

static ERROR_INVALID_LOOT_TABLE: CommandErrorType<1> = CommandErrorType::new(
    translation::bedrock::COMMANDS_LOOT_FAILURE_INVALIDLOOTTABLE,
    "Loot table '%s' not found",
);

static ERROR_ENTITY_NO_LOOT_TABLE: CommandErrorType<1> = CommandErrorType::new(
    translation::bedrock::COMMANDS_LOOT_FAILURE_ENTITYNOLOOTTABLE,
    "Entity %s has no loot table",
);

static ERROR_NO_CONTAINER: CommandErrorType<1> = CommandErrorType::new(
    translation::bedrock::COMMANDS_LOOT_FAILURE_NOCONTAINER,
    "Target position %s is not a container",
);

#[derive(Clone, Copy)]
enum Target {
    Give,
    Spawn,
    Insert,
}

#[derive(Clone, Copy)]
enum Source {
    Loot,
    Kill,
    Mine { has_tool: bool },
}

struct LootExecutor {
    target: Target,
    source: Source,
}

async fn insert_into_inventory(
    inventory: &dyn pumpkin_world::inventory::Inventory,
    mut stack: pumpkin_data::item_stack::ItemStack,
) -> pumpkin_data::item_stack::ItemStack {
    for i in 0..inventory.size() {
        if stack.is_empty() {
            break;
        }
        let mut slot_stack = inventory.get_stack(i).await;
        if slot_stack.are_items_and_components_equal(&stack) && slot_stack.is_stackable() {
            let max_stack_size = slot_stack.get_max_stack_size();
            let space = max_stack_size.saturating_sub(slot_stack.item_count);
            if space > 0 {
                let to_add = stack.item_count.min(space);
                slot_stack.item_count += to_add;
                stack.item_count -= to_add;
                inventory.set_stack(i, slot_stack).await;
            }
        }
    }

    for i in 0..inventory.size() {
        if stack.is_empty() {
            break;
        }
        let slot_stack = inventory.get_stack(i).await;
        if slot_stack.is_empty() {
            let to_add = stack.item_count.min(stack.get_max_stack_size());
            inventory.set_stack(i, stack.copy_with_count(to_add)).await;
            stack.item_count -= to_add;
        }
    }

    stack
}

impl CommandExecutor for LootExecutor {
    #[allow(clippy::too_many_lines)]
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            let mut stacks = Vec::new();

            match self.source {
                Source::Loot => {
                    let loot_table_str = StringArgumentType::get(context, "loot_table")?;
                    let formatted_key = if loot_table_str.contains(':') {
                        loot_table_str.to_string()
                    } else {
                        format!("minecraft:{loot_table_str}")
                    };
                    let world = context.world();
                    let world_time = world.level_info.load().day_time as u64;
                    let seed =
                        derive_loot_seed(world.level.seed.0, None, world_time, 0x434f4d4d414e44)
                            as i64;

                    // A datapack override wins over the generated vanilla
                    // chest registry.  Invalid runtime shapes are reported
                    // as an invalid table instead of silently falling back to
                    // a different table with the same key.
                    let datapack_result = if let Some(server) = world.server.upgrade() {
                        let resources = server.recipe_manager.datapack_resources().await;
                        resources.loot_tables.contains_key(&formatted_key).then(|| {
                            crate::world::loot::generate_datapack_chest_loot(
                                &resources,
                                &formatted_key,
                                seed,
                            )
                        })
                    } else {
                        None
                    };
                    match datapack_result {
                        Some(Ok(items)) => stacks = items,
                        Some(Err(error)) => {
                            tracing::warn!(
                                "Could not evaluate datapack loot table {formatted_key}: {error}"
                            );
                            return Err(ERROR_INVALID_LOOT_TABLE.create_without_context(
                                TextComponent::text(loot_table_str.to_string()),
                            ));
                        }
                        None => {
                            let Some(table) = pumpkin_data::chest_loot_table::get_chest_loot_table(
                                &formatted_key,
                            ) else {
                                return Err(ERROR_INVALID_LOOT_TABLE.create_without_context(
                                    TextComponent::text(loot_table_str.to_string()),
                                ));
                            };
                            stacks = crate::world::loot::generate_chest_loot(table, seed);
                        }
                    }
                }
                Source::Kill => {
                    let target_entities =
                        EntityArgumentType::get_entities(context, "target_entity").await?;
                    let mut has_loot = false;
                    for entity in target_entities {
                        if let Some(loot_table) = &entity.get_entity().entity_type.loot_table {
                            has_loot = true;
                            let entity_position = entity.get_entity().pos.load();
                            let entity_uuid = entity.get_entity().entity_uuid.as_u128();
                            let world = context.world();
                            let world_time = world.level_info.load().day_time as u64;
                            let mut params = crate::world::loot::LootContextParameters {
                                biome: Some(
                                    world
                                        .get_biome(&entity.get_entity().block_pos.load())
                                        .registry_id,
                                ),
                                position: Some(entity_position),
                                world_time,
                                random_seed: Some(derive_loot_seed(
                                    world.level.seed.0,
                                    Some(entity_position),
                                    world_time,
                                    (entity_uuid as u64)
                                        ^ ((entity_uuid >> 64) as u64)
                                        ^ 0x434d444b494c4c,
                                )),
                                ..Default::default()
                            };
                            params.attach_biome_resolver(&world);
                            stacks.extend(loot_table.get_loot(params));
                        }
                    }
                    if !has_loot {
                        let entity_name = "selected entity".to_string();
                        return Err(ERROR_ENTITY_NO_LOOT_TABLE
                            .create_without_context(TextComponent::text(entity_name)));
                    }
                }
                Source::Mine { has_tool } => {
                    let pos = BlockPosArgumentType::get_block_pos(context, "mine_pos")
                        .or_else(|_| BlockPosArgumentType::get_block_pos(context, "pos"))?;
                    let world = context.world();
                    let block = world.get_block(&pos);

                    if let Some(loot_table) = &block.loot_table {
                        let tool_item = if has_tool {
                            let tool_str = StringArgumentType::get(context, "tool")?;
                            let key = tool_str.strip_prefix("minecraft:").unwrap_or(tool_str);
                            pumpkin_data::item::Item::from_registry_key(key)
                        } else {
                            None
                        };

                        let tool_stack =
                            tool_item.map(|item| pumpkin_data::item_stack::ItemStack::new(1, item));

                        let mut params = crate::world::loot::LootContextParameters {
                            block_state: Some(world.get_block_state(&pos)),
                            biome: Some(world.get_biome(&pos).registry_id),
                            tool: tool_stack,
                            position: Some(pos.to_centered_f64()),
                            world_time: world.level_info.load().day_time as u64,
                            random_seed: Some(derive_loot_seed(
                                world.level.seed.0,
                                Some(pos.to_centered_f64()),
                                world.level_info.load().day_time as u64,
                                0x434d444d494e45,
                            )),
                            ..Default::default()
                        };
                        params.attach_biome_resolver(&world);

                        stacks.extend(loot_table.get_loot(params));
                    }
                }
            }

            let total_items: i32 = stacks.iter().map(|s| s.item_count as i32).sum();

            match self.target {
                Target::Give => {
                    let targets = EntityArgumentType::get_players(context, "targets").await?;
                    for player in &targets {
                        for stack in &stacks {
                            let mut remaining = stack.clone();
                            player.inventory.insert_stack_anywhere(&mut remaining).await;
                            if !remaining.is_empty() {
                                player.drop_item(remaining).await;
                            }
                        }
                    }
                }
                Target::Spawn => {
                    let pos = BlockPosArgumentType::get_block_pos(context, "pos")?;
                    let world = context.world();
                    for stack in stacks {
                        world.drop_stack(&pos, stack).await;
                    }
                }
                Target::Insert => {
                    let pos = BlockPosArgumentType::get_block_pos(context, "pos")?;
                    let world = context.world();
                    if let Some(block_entity) = world.get_block_entity(&pos) {
                        if let Some(inventory) = block_entity.get_inventory() {
                            for stack in stacks {
                                let remaining =
                                    insert_into_inventory(inventory.as_ref(), stack).await;
                                if !remaining.is_empty() {
                                    world.drop_stack(&pos, remaining).await;
                                }
                            }
                        } else {
                            return Err(ERROR_NO_CONTAINER
                                .create_without_context(TextComponent::text(pos.to_string())));
                        }
                    } else {
                        return Err(ERROR_NO_CONTAINER
                            .create_without_context(TextComponent::text(pos.to_string())));
                    }
                }
            }

            let msg = TextComponent::translate_cross(
                translation::bedrock::COMMANDS_LOOT_SUCCESS,
                translation::bedrock::COMMANDS_LOOT_SUCCESS,
                [TextComponent::text(total_items.to_string())],
            );
            context.source.send_feedback(msg, true).await;

            Ok(total_items)
        })
    }
}

#[expect(clippy::too_many_lines)]
pub fn register(dispatcher: &mut CommandDispatcher, registry: &mut PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    let builder = command("loot", DESCRIPTION)
        .requires(PERMISSION)
        // give <targets>
        .then(
            literal("give").then(
                argument("targets", EntityArgumentType::Players)
                    .then(literal("loot").then(
                        argument("loot_table", StringArgumentType::SingleWord).executes(
                            LootExecutor {
                                target: Target::Give,
                                source: Source::Loot,
                            },
                        ),
                    ))
                    .then(literal("kill").then(
                        argument("target_entity", EntityArgumentType::Entities).executes(
                            LootExecutor {
                                target: Target::Give,
                                source: Source::Kill,
                            },
                        ),
                    ))
                    .then(
                        literal("mine").then(
                            argument("pos", BlockPosArgumentType)
                                .executes(LootExecutor {
                                    target: Target::Give,
                                    source: Source::Mine { has_tool: false },
                                })
                                .then(argument("tool", StringArgumentType::SingleWord).executes(
                                    LootExecutor {
                                        target: Target::Give,
                                        source: Source::Mine { has_tool: true },
                                    },
                                )),
                        ),
                    ),
            ),
        )
        // spawn <pos>
        .then(
            literal("spawn").then(
                argument("pos", BlockPosArgumentType)
                    .then(literal("loot").then(
                        argument("loot_table", StringArgumentType::SingleWord).executes(
                            LootExecutor {
                                target: Target::Spawn,
                                source: Source::Loot,
                            },
                        ),
                    ))
                    .then(literal("kill").then(
                        argument("target_entity", EntityArgumentType::Entities).executes(
                            LootExecutor {
                                target: Target::Spawn,
                                source: Source::Kill,
                            },
                        ),
                    ))
                    .then(
                        literal("mine").then(
                            argument("mine_pos", BlockPosArgumentType)
                                .executes(LootExecutor {
                                    target: Target::Spawn,
                                    source: Source::Mine { has_tool: false },
                                })
                                .then(argument("tool", StringArgumentType::SingleWord).executes(
                                    LootExecutor {
                                        target: Target::Spawn,
                                        source: Source::Mine { has_tool: true },
                                    },
                                )),
                        ),
                    ),
            ),
        )
        // insert <pos>
        .then(
            literal("insert").then(
                argument("pos", BlockPosArgumentType)
                    .then(literal("loot").then(
                        argument("loot_table", StringArgumentType::SingleWord).executes(
                            LootExecutor {
                                target: Target::Insert,
                                source: Source::Loot,
                            },
                        ),
                    ))
                    .then(literal("kill").then(
                        argument("target_entity", EntityArgumentType::Entities).executes(
                            LootExecutor {
                                target: Target::Insert,
                                source: Source::Kill,
                            },
                        ),
                    ))
                    .then(
                        literal("mine").then(
                            argument("mine_pos", BlockPosArgumentType)
                                .executes(LootExecutor {
                                    target: Target::Insert,
                                    source: Source::Mine { has_tool: false },
                                })
                                .then(argument("tool", StringArgumentType::SingleWord).executes(
                                    LootExecutor {
                                        target: Target::Insert,
                                        source: Source::Mine { has_tool: true },
                                    },
                                )),
                        ),
                    ),
            ),
        );

    dispatcher.register(builder);
}
