use crate::block::entities::block_entity_from_nbt;
use crate::command::CommandResult;
use crate::command::args::bounded_num::BoundedNumArgumentConsumer;
use crate::command::args::entity::EntityArgumentConsumer;
use crate::command::args::nbt::NbtCompoundArgumentConsumer;
use crate::command::args::position_block::BlockPosArgumentConsumer;
use crate::command::args::{ArgumentConsumer, ConsumeResult, GetClientSideArgParser};
use crate::command::tree::builder::literal;
use crate::command::{
    CommandError, CommandExecutor, CommandSender,
    args::{Arg, ConsumedArgs, FindArg},
    tree::{CommandTree, RawArgs, builder::argument},
};
use crate::entity::NBTStorage;
use CommandError::InvalidConsumption;
use pumpkin_data::translation;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::java::client::play::{ArgumentType, SuggestionProviders};
use pumpkin_util::text::TextComponent;
use pumpkin_util::text::color::NamedColor;

const NAMES: [&str; 1] = ["data"];
const DESCRIPTION: &str = "Query and modify data of entities and blocks";

const ARG_ENTITY: &str = "entity";
const ARG_BLOCK_POS: &str = "targetPos";
const ARG_NBT: &str = "nbt";
const ARG_PATH: &str = "path";
const ARG_SCALE: &str = "scale";

const fn scale_consumer() -> BoundedNumArgumentConsumer<f64> {
    BoundedNumArgumentConsumer::new()
}

pub struct NbtPathArgumentConsumer;

impl GetClientSideArgParser for NbtPathArgumentConsumer {
    fn get_client_side_parser(&self) -> ArgumentType {
        ArgumentType::NbtPath
    }

    fn get_client_side_suggestion_type_override(&self) -> Option<SuggestionProviders> {
        None
    }
}

impl ArgumentConsumer for NbtPathArgumentConsumer {
    fn consume<'a, 'b>(
        &'a self,
        _sender: &'a CommandSender,
        _server: &'a crate::server::Server,
        args: &'b mut RawArgs<'a>,
    ) -> ConsumeResult<'a> {
        let s_opt: Option<&'a str> = args.pop().map(|arg| arg.value);
        Box::pin(async move { s_opt.map(Arg::Simple) })
    }
}

impl<'a> FindArg<'a> for NbtPathArgumentConsumer {
    type Data = &'a str;

    fn find_arg(args: &'a ConsumedArgs, name: &str) -> Result<Self::Data, CommandError> {
        match args.get(name) {
            Some(Arg::Simple(data)) => Ok(data),
            _ => Err(CommandError::InvalidConsumption(Some(name.to_string()))),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NbtPathSegment {
    Key(String),
    Index(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct NbtPath {
    pub raw: String,
    pub segments: Vec<NbtPathSegment>,
}

impl NbtPath {
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut segments = Vec::new();
        let mut chars = input.chars().peekable();

        while let Some(&ch) = chars.peek() {
            if ch == '.' {
                chars.next();
                continue;
            }
            if ch == '[' {
                chars.next();
                let mut idx_str = String::new();
                while let Some(&c) = chars.peek() {
                    if c == ']' {
                        chars.next();
                        break;
                    }
                    idx_str.push(c);
                    chars.next();
                }
                let idx = idx_str
                    .parse::<usize>()
                    .map_err(|e| format!("Invalid index: {e}"))?;
                segments.push(NbtPathSegment::Index(idx));
            } else {
                let mut key = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '.' || c == '[' {
                        break;
                    }
                    key.push(c);
                    chars.next();
                }
                if !key.is_empty() {
                    segments.push(NbtPathSegment::Key(key));
                }
            }
        }

        if segments.is_empty() {
            return Err("Empty NBT path".into());
        }

        Ok(Self {
            raw: input.to_string(),
            segments,
        })
    }

    pub fn query<'a>(&self, root: &'a NbtTag) -> Result<&'a NbtTag, CommandError> {
        let mut curr = root;
        for segment in &self.segments {
            match segment {
                NbtPathSegment::Key(key) => {
                    let NbtTag::Compound(compound) = curr else {
                        return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                            translation::java::COMMANDS_DATA_GET_UNKNOWN,
                            translation::java::COMMANDS_DATA_GET_UNKNOWN,
                            [],
                        )));
                    };
                    let Some(child) = compound.child_tags.get(key.as_str()) else {
                        return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                            translation::java::COMMANDS_DATA_GET_UNKNOWN,
                            translation::java::COMMANDS_DATA_GET_UNKNOWN,
                            [],
                        )));
                    };
                    curr = child;
                }
                NbtPathSegment::Index(idx) => {
                    let NbtTag::List(list) = curr else {
                        return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                            translation::java::COMMANDS_DATA_GET_UNKNOWN,
                            translation::java::COMMANDS_DATA_GET_UNKNOWN,
                            [],
                        )));
                    };
                    let Some(child) = list.get(*idx) else {
                        return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                            translation::java::COMMANDS_DATA_GET_UNKNOWN,
                            translation::java::COMMANDS_DATA_GET_UNKNOWN,
                            [],
                        )));
                    };
                    curr = child;
                }
            }
        }
        Ok(curr)
    }

    pub fn remove(&self, root: &mut NbtCompound) -> bool {
        if self.segments.is_empty() {
            return false;
        }
        if self.segments.len() == 1 {
            return match &self.segments[0] {
                NbtPathSegment::Key(key) => root.child_tags.remove(key.as_str()).is_some(),
                NbtPathSegment::Index(_) => false,
            };
        }

        let first = &self.segments[0];
        let mut curr: &mut NbtTag = match first {
            NbtPathSegment::Key(k) => match root.child_tags.get_mut(k.as_str()) {
                Some(tag) => tag,
                None => return false,
            },
            NbtPathSegment::Index(_) => return false,
        };

        for segment in &self.segments[1..self.segments.len() - 1] {
            match segment {
                NbtPathSegment::Key(key) => {
                    let NbtTag::Compound(comp) = curr else {
                        return false;
                    };
                    let Some(next) = comp.child_tags.get_mut(key.as_str()) else {
                        return false;
                    };
                    curr = next;
                }
                NbtPathSegment::Index(idx) => {
                    let NbtTag::List(list) = curr else {
                        return false;
                    };
                    let Some(next) = list.get_mut(*idx) else {
                        return false;
                    };
                    curr = next;
                }
            }
        }

        let last = &self.segments[self.segments.len() - 1];
        match last {
            NbtPathSegment::Key(key) => {
                let NbtTag::Compound(comp) = curr else {
                    return false;
                };
                comp.child_tags.remove(key.as_str()).is_some()
            }
            NbtPathSegment::Index(idx) => {
                let NbtTag::List(list) = curr else {
                    return false;
                };
                if *idx < list.len() {
                    list.remove(*idx);
                    true
                } else {
                    false
                }
            }
        }
    }
}

fn get_numeric_value(tag: &NbtTag) -> Option<f64> {
    match tag {
        NbtTag::Byte(b) => Some(*b as f64),
        NbtTag::Short(s) => Some(*s as f64),
        NbtTag::Int(i) => Some(*i as f64),
        NbtTag::Long(l) => Some(*l as f64),
        NbtTag::Float(f) => Some(*f as f64),
        NbtTag::Double(d) => Some(*d),
        _ => None,
    }
}

struct GetEntityDataExecutor;

impl CommandExecutor for GetEntityDataExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        _server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let Some(Arg::Entity(entity)) = args.get(&ARG_ENTITY) else {
                return Err(InvalidConsumption(Some(ARG_ENTITY.into())));
            };
            display_data(
                entity.as_nbt_storage(),
                entity.get_display_name().await,
                sender,
            )
            .await
        })
    }
}

#[expect(clippy::too_many_lines)]
pub fn snbt_colorful_display(tag: &NbtTag, depth: usize) -> Result<TextComponent, String> {
    let folded = TextComponent::text("<...>").color_named(NamedColor::Gray);
    match tag {
        NbtTag::End => Err("Unexpected end tag".into()),
        NbtTag::Byte(value) => {
            let byte_format = TextComponent::text("b").color_named(NamedColor::Red);
            Ok(TextComponent::text(format!("{value}"))
                .color_named(NamedColor::Gold)
                .add_child(byte_format))
        }
        NbtTag::Short(value) => {
            let short_format = TextComponent::text("s").color_named(NamedColor::Red);
            Ok(TextComponent::text(format!("{value}"))
                .color_named(NamedColor::Gold)
                .add_child(short_format))
        }
        NbtTag::Int(value) => {
            Ok(TextComponent::text(format!("{value}")).color_named(NamedColor::Gold))
        }
        NbtTag::Long(value) => {
            let long_format = TextComponent::text("L").color_named(NamedColor::Red);
            Ok(TextComponent::text(format!("{value}"))
                .color_named(NamedColor::Gold)
                .add_child(long_format))
        }
        NbtTag::Float(value) => {
            let float_format = TextComponent::text("f").color_named(NamedColor::Red);
            Ok(TextComponent::text(format!("{value}"))
                .color_named(NamedColor::Gold)
                .add_child(float_format))
        }
        NbtTag::Double(value) => {
            let double_format = TextComponent::text("d").color_named(NamedColor::Red);
            Ok(TextComponent::text(format!("{value}"))
                .color_named(NamedColor::Gold)
                .add_child(double_format))
        }
        NbtTag::ByteArray(value) => {
            let byte_array_format = TextComponent::text("B").color_named(NamedColor::Red);
            let mut content = TextComponent::text("[")
                .add_child(byte_array_format.clone())
                .add_child(TextComponent::text("; "));

            for (index, byte) in value.iter().take(128).enumerate() {
                content = content
                    .add_child(TextComponent::text(format!("{byte}")))
                    .add_child(byte_array_format.clone());
                if index < value.len() - 1 {
                    content = content.add_child(TextComponent::text(", "));
                }
            }

            if value.len() > 128 {
                content = content.add_child(folded);
            }

            content = content.add_child(TextComponent::text("]"));
            Ok(content)
        }
        NbtTag::String(value) => {
            let escaped_value = value
                .replace('"', "\\\"")
                .replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('\t', "\\t")
                .replace('\r', "\\r")
                .replace('\x0c', "\\f")
                .replace('\x08', "\\b");

            Ok(TextComponent::text(format!("\"{escaped_value}\"")).color_named(NamedColor::Green))
        }
        NbtTag::List(value) => {
            if value.is_empty() {
                Ok(TextComponent::text("[]"))
            } else if depth >= 64 {
                Ok(TextComponent::text("[")
                    .add_child(folded)
                    .add_child(TextComponent::text("]")))
            } else {
                let mut content = TextComponent::text("[");

                for (index, item) in value.iter().take(128).enumerate() {
                    let item_display = snbt_colorful_display(item, depth + 1)
                        .map_err(|string| format!("Error displaying item.[{index}]: {string}"))?;
                    content = content.add_child(item_display);

                    if index < value.len() - 1 {
                        content = content.add_child(TextComponent::text(", "));
                    }
                }

                if value.len() > 128 {
                    content = content.add_child(folded);
                }

                content = content.add_child(TextComponent::text("]"));
                Ok(content)
            }
        }
        NbtTag::Compound(value) => {
            if value.is_empty() {
                Ok(TextComponent::text("{}"))
            } else if depth >= 64 {
                Ok(TextComponent::text("{")
                    .add_child(folded)
                    .add_child(TextComponent::text("}")))
            } else {
                let mut content = TextComponent::text("{");

                for (index, (key, item)) in value.child_tags.iter().take(128).enumerate() {
                    let item_display = snbt_colorful_display(item, depth + 1)
                        .map_err(|string| format!("Error displaying item.{key}: {string}"))?;
                    content = content
                        .add_child(
                            TextComponent::text(key.to_string()).color_named(NamedColor::Aqua),
                        )
                        .add_child(TextComponent::text(": "))
                        .add_child(item_display);

                    if index < value.child_tags.len() - 1 {
                        content = content.add_child(TextComponent::text(", "));
                    }
                }

                if value.child_tags.len() > 128 {
                    content = content.add_child(folded);
                }

                content = content.add_child(TextComponent::text("}"));
                Ok(content)
            }
        }
        NbtTag::IntArray(value) => {
            let int_array_format = TextComponent::text("I").color_named(NamedColor::Red);
            let mut content = TextComponent::text("[")
                .add_child(int_array_format)
                .add_child(TextComponent::text("; "));

            for (index, int) in value.iter().take(128).enumerate() {
                content = content
                    .add_child(TextComponent::text(format!("{int}")).color_named(NamedColor::Gold));
                if index < value.len() - 1 {
                    content = content.add_child(TextComponent::text(", "));
                }
            }

            if value.len() > 128 {
                content = content.add_child(folded);
            }

            content = content.add_child(TextComponent::text("]"));
            Ok(content)
        }
        NbtTag::LongArray(value) => {
            let long_array_format = TextComponent::text("L").color_named(NamedColor::Red);
            let mut content = TextComponent::text("[")
                .add_child(long_array_format.clone())
                .add_child(TextComponent::text("; "));

            for (index, long) in value.iter().take(128).enumerate() {
                content = content
                    .add_child(TextComponent::text(format!("{long}")))
                    .add_child(long_array_format.clone());
                if index < value.len() - 1 {
                    content = content.add_child(TextComponent::text(", "));
                }
            }

            if value.len() > 128 {
                content = content.add_child(folded);
            }

            content = content.add_child(TextComponent::text("]"));
            Ok(content)
        }
    }
}

async fn display_data(
    storage: &dyn NBTStorage,
    target_name: TextComponent,
    sender: &CommandSender,
) -> Result<i32, CommandError> {
    let mut nbt = NbtCompound::new();
    storage.write_nbt(&mut nbt).await;
    let tag = NbtTag::Compound(nbt);

    let result = get_i32_result(&tag)?;
    let display = snbt_colorful_display(&tag, 0)
        .map_err(|string| CommandError::CommandFailed(TextComponent::text(string)))?;
    sender
        .send_message(TextComponent::translate_cross(
            translation::java::COMMANDS_DATA_ENTITY_QUERY,
            translation::java::COMMANDS_DATA_ENTITY_QUERY,
            [target_name, display],
        ))
        .await;

    Ok(result)
}

fn get_i32_result(tag: &NbtTag) -> Result<i32, CommandError> {
    match tag {
        NbtTag::End => Err(CommandError::CommandFailed(TextComponent::translate_cross(
            translation::java::COMMANDS_DATA_GET_UNKNOWN,
            translation::java::COMMANDS_DATA_GET_UNKNOWN,
            [],
        ))),

        NbtTag::Byte(b) => Ok(*b as i32),
        NbtTag::Short(s) => Ok(*s as i32),
        NbtTag::Int(i) => Ok(*i),
        NbtTag::Long(l) => Ok((*l).clamp(i32::MIN as i64, i32::MAX as i64) as i32),
        NbtTag::Float(f) => Ok({
            let i = *f as i32;
            if *f < i as f32 { i - 1 } else { i }
        }),
        NbtTag::Double(d) => Ok({
            let i = *d as i32;
            if *d < i as f64 { i - 1 } else { i }
        }),

        NbtTag::ByteArray(items) => Ok(items.len() as i32),
        NbtTag::IntArray(items) => Ok(items.len() as i32),
        NbtTag::LongArray(items) => Ok(items.len() as i32),

        NbtTag::String(string) => Ok(string.len() as i32),
        NbtTag::List(nbt_tags) => Ok(nbt_tags.len() as i32),
        NbtTag::Compound(nbt_compound) => Ok(nbt_compound.child_tags.len() as i32),
    }
}

pub fn merge_nbt_compound(target: &mut NbtCompound, other: &NbtCompound) {
    for (key, other_tag) in &other.child_tags {
        match other_tag {
            NbtTag::Compound(other_child) => {
                if let Some(NbtTag::Compound(target_child)) = target.child_tags.get_mut(key) {
                    merge_nbt_compound(target_child, other_child);
                } else {
                    target.child_tags.insert(key.clone(), other_tag.clone());
                }
            }
            _ => {
                target.child_tags.insert(key.clone(), other_tag.clone());
            }
        }
    }
}

struct GetBlockDataExecutor;

impl CommandExecutor for GetBlockDataExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let world = sender
                .world_or_first(server)
                .ok_or(CommandError::InvalidRequirement)?;
            let pos = BlockPosArgumentConsumer::find_loaded_arg(args, ARG_BLOCK_POS, &world)?;
            let Some(block_entity) = world.get_block_entity(&pos) else {
                return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_BLOCK_INVALID,
                    translation::java::COMMANDS_DATA_BLOCK_INVALID,
                    [],
                )));
            };

            let mut nbt = NbtCompound::new();
            block_entity.write_internal(&mut nbt).await;
            let tag = NbtTag::Compound(nbt);

            let result = get_i32_result(&tag)?;
            let display = snbt_colorful_display(&tag, 0)
                .map_err(|string| CommandError::CommandFailed(TextComponent::text(string)))?;

            sender
                .send_message(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_BLOCK_QUERY,
                    translation::java::COMMANDS_DATA_BLOCK_QUERY,
                    [
                        TextComponent::text(pos.0.x.to_string()),
                        TextComponent::text(pos.0.y.to_string()),
                        TextComponent::text(pos.0.z.to_string()),
                        display,
                    ],
                ))
                .await;

            Ok(result)
        })
    }
}

struct MergeBlockDataExecutor;

impl CommandExecutor for MergeBlockDataExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let world = sender
                .world_or_first(server)
                .ok_or(CommandError::InvalidRequirement)?;
            let pos = BlockPosArgumentConsumer::find_loaded_arg(args, ARG_BLOCK_POS, &world)?;
            let nbt_to_merge = NbtCompoundArgumentConsumer::find_arg(args, ARG_NBT)?;

            let Some(block_entity) = world.get_block_entity(&pos) else {
                return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_BLOCK_INVALID,
                    translation::java::COMMANDS_DATA_BLOCK_INVALID,
                    [],
                )));
            };

            let mut old_nbt = NbtCompound::new();
            block_entity.write_internal(&mut old_nbt).await;

            let mut merged_nbt = old_nbt.clone();
            merge_nbt_compound(&mut merged_nbt, &nbt_to_merge);

            if old_nbt == merged_nbt {
                return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_MERGE_FAILED,
                    translation::java::COMMANDS_DATA_MERGE_FAILED,
                    [],
                )));
            }

            if let Some(new_block_entity) = block_entity_from_nbt(&merged_nbt) {
                world.add_block_entity(new_block_entity);
            } else {
                world.add_block_entity_nbt(pos, &merged_nbt);
            }

            sender
                .send_message(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_BLOCK_MODIFIED,
                    translation::java::COMMANDS_DATA_BLOCK_MODIFIED,
                    [
                        TextComponent::text(pos.0.x.to_string()),
                        TextComponent::text(pos.0.y.to_string()),
                        TextComponent::text(pos.0.z.to_string()),
                    ],
                ))
                .await;

            Ok(1)
        })
    }
}

struct MergeEntityDataExecutor;

impl CommandExecutor for MergeEntityDataExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        _server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let Some(Arg::Entity(entity)) = args.get(&ARG_ENTITY) else {
                return Err(InvalidConsumption(Some(ARG_ENTITY.into())));
            };
            let nbt_to_merge = NbtCompoundArgumentConsumer::find_arg(args, ARG_NBT)?;

            let storage = entity.as_nbt_storage();
            let mut old_nbt = NbtCompound::new();
            storage.write_nbt(&mut old_nbt).await;

            let mut merged_nbt = old_nbt.clone();
            merge_nbt_compound(&mut merged_nbt, &nbt_to_merge);

            if old_nbt == merged_nbt {
                return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_MERGE_FAILED,
                    translation::java::COMMANDS_DATA_MERGE_FAILED,
                    [],
                )));
            }

            storage.read_nbt_non_mut(&merged_nbt).await;

            let display_name = entity.get_display_name().await;
            sender
                .send_message(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_ENTITY_MODIFIED,
                    translation::java::COMMANDS_DATA_ENTITY_MODIFIED,
                    [display_name],
                ))
                .await;

            Ok(1)
        })
    }
}

struct GetEntityPathDataExecutor;

impl CommandExecutor for GetEntityPathDataExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        _server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let Some(Arg::Entity(entity)) = args.get(&ARG_ENTITY) else {
                return Err(InvalidConsumption(Some(ARG_ENTITY.into())));
            };
            let path_str = NbtPathArgumentConsumer::find_arg(args, ARG_PATH)?;
            let path = NbtPath::parse(path_str)
                .map_err(|e| CommandError::CommandFailed(TextComponent::text(e)))?;

            let mut nbt = NbtCompound::new();
            entity.as_nbt_storage().write_nbt(&mut nbt).await;
            let root = NbtTag::Compound(nbt);
            let tag = path.query(&root)?;

            let result = get_i32_result(tag)?;
            let display = snbt_colorful_display(tag, 0)
                .map_err(|string| CommandError::CommandFailed(TextComponent::text(string)))?;
            let display_name = entity.get_display_name().await;

            sender
                .send_message(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_ENTITY_QUERY,
                    translation::java::COMMANDS_DATA_ENTITY_QUERY,
                    [display_name, display],
                ))
                .await;

            Ok(result)
        })
    }
}

struct GetEntityPathScaleDataExecutor;

impl CommandExecutor for GetEntityPathScaleDataExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        _server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let Some(Arg::Entity(entity)) = args.get(&ARG_ENTITY) else {
                return Err(InvalidConsumption(Some(ARG_ENTITY.into())));
            };
            let path_str = NbtPathArgumentConsumer::find_arg(args, ARG_PATH)?;
            let path = NbtPath::parse(path_str)
                .map_err(|e| CommandError::CommandFailed(TextComponent::text(e)))?;
            let scale: f64 = BoundedNumArgumentConsumer::<f64>::find_arg(args, ARG_SCALE)??;

            let mut nbt = NbtCompound::new();
            entity.as_nbt_storage().write_nbt(&mut nbt).await;
            let root = NbtTag::Compound(nbt);
            let tag = path.query(&root)?;

            let Some(num) = get_numeric_value(tag) else {
                return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_GET_INVALID,
                    translation::java::COMMANDS_DATA_GET_INVALID,
                    [TextComponent::text(path.raw.clone())],
                )));
            };

            let scaled = (num * scale).floor() as i32;
            let display_name = entity.get_display_name().await;

            sender
                .send_message(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_ENTITY_GET,
                    translation::java::COMMANDS_DATA_ENTITY_GET,
                    [
                        TextComponent::text(path.raw),
                        display_name,
                        TextComponent::text(format!("{scale:.2}")),
                        TextComponent::text(scaled.to_string()),
                    ],
                ))
                .await;

            Ok(scaled)
        })
    }
}

struct GetBlockPathDataExecutor;

impl CommandExecutor for GetBlockPathDataExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let world = sender
                .world_or_first(server)
                .ok_or(CommandError::InvalidRequirement)?;
            let pos = BlockPosArgumentConsumer::find_loaded_arg(args, ARG_BLOCK_POS, &world)?;
            let Some(block_entity) = world.get_block_entity(&pos) else {
                return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_BLOCK_INVALID,
                    translation::java::COMMANDS_DATA_BLOCK_INVALID,
                    [],
                )));
            };
            let path_str = NbtPathArgumentConsumer::find_arg(args, ARG_PATH)?;
            let path = NbtPath::parse(path_str)
                .map_err(|e| CommandError::CommandFailed(TextComponent::text(e)))?;

            let mut nbt = NbtCompound::new();
            block_entity.write_internal(&mut nbt).await;
            let root = NbtTag::Compound(nbt);
            let tag = path.query(&root)?;

            let result = get_i32_result(tag)?;
            let display = snbt_colorful_display(tag, 0)
                .map_err(|string| CommandError::CommandFailed(TextComponent::text(string)))?;

            sender
                .send_message(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_BLOCK_QUERY,
                    translation::java::COMMANDS_DATA_BLOCK_QUERY,
                    [
                        TextComponent::text(pos.0.x.to_string()),
                        TextComponent::text(pos.0.y.to_string()),
                        TextComponent::text(pos.0.z.to_string()),
                        display,
                    ],
                ))
                .await;

            Ok(result)
        })
    }
}

struct GetBlockPathScaleDataExecutor;

impl CommandExecutor for GetBlockPathScaleDataExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let world = sender
                .world_or_first(server)
                .ok_or(CommandError::InvalidRequirement)?;
            let pos = BlockPosArgumentConsumer::find_loaded_arg(args, ARG_BLOCK_POS, &world)?;
            let Some(block_entity) = world.get_block_entity(&pos) else {
                return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_BLOCK_INVALID,
                    translation::java::COMMANDS_DATA_BLOCK_INVALID,
                    [],
                )));
            };
            let path_str = NbtPathArgumentConsumer::find_arg(args, ARG_PATH)?;
            let path = NbtPath::parse(path_str)
                .map_err(|e| CommandError::CommandFailed(TextComponent::text(e)))?;
            let scale: f64 = BoundedNumArgumentConsumer::<f64>::find_arg(args, ARG_SCALE)??;

            let mut nbt = NbtCompound::new();
            block_entity.write_internal(&mut nbt).await;
            let root = NbtTag::Compound(nbt);
            let tag = path.query(&root)?;

            let Some(num) = get_numeric_value(tag) else {
                return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_GET_INVALID,
                    translation::java::COMMANDS_DATA_GET_INVALID,
                    [TextComponent::text(path.raw.clone())],
                )));
            };

            let scaled = (num * scale).floor() as i32;

            sender
                .send_message(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_BLOCK_GET,
                    translation::java::COMMANDS_DATA_BLOCK_GET,
                    [
                        TextComponent::text(path.raw),
                        TextComponent::text(pos.0.x.to_string()),
                        TextComponent::text(pos.0.y.to_string()),
                        TextComponent::text(pos.0.z.to_string()),
                        TextComponent::text(format!("{scale:.2}")),
                        TextComponent::text(scaled.to_string()),
                    ],
                ))
                .await;

            Ok(scaled)
        })
    }
}

struct RemoveEntityDataExecutor;

impl CommandExecutor for RemoveEntityDataExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        _server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let Some(Arg::Entity(entity)) = args.get(&ARG_ENTITY) else {
                return Err(InvalidConsumption(Some(ARG_ENTITY.into())));
            };
            let path_str = NbtPathArgumentConsumer::find_arg(args, ARG_PATH)?;
            let path = NbtPath::parse(path_str)
                .map_err(|e| CommandError::CommandFailed(TextComponent::text(e)))?;

            let storage = entity.as_nbt_storage();
            let mut nbt = NbtCompound::new();
            storage.write_nbt(&mut nbt).await;

            let removed = path.remove(&mut nbt);
            if !removed {
                return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_MERGE_FAILED,
                    translation::java::COMMANDS_DATA_MERGE_FAILED,
                    [],
                )));
            }

            storage.read_nbt_non_mut(&nbt).await;

            let display_name = entity.get_display_name().await;
            sender
                .send_message(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_ENTITY_MODIFIED,
                    translation::java::COMMANDS_DATA_ENTITY_MODIFIED,
                    [display_name],
                ))
                .await;

            Ok(1)
        })
    }
}

struct RemoveBlockDataExecutor;

impl CommandExecutor for RemoveBlockDataExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let world = sender
                .world_or_first(server)
                .ok_or(CommandError::InvalidRequirement)?;
            let pos = BlockPosArgumentConsumer::find_loaded_arg(args, ARG_BLOCK_POS, &world)?;
            let Some(block_entity) = world.get_block_entity(&pos) else {
                return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_BLOCK_INVALID,
                    translation::java::COMMANDS_DATA_BLOCK_INVALID,
                    [],
                )));
            };

            let path_str = NbtPathArgumentConsumer::find_arg(args, ARG_PATH)?;
            let path = NbtPath::parse(path_str)
                .map_err(|e| CommandError::CommandFailed(TextComponent::text(e)))?;

            let mut nbt = NbtCompound::new();
            block_entity.write_internal(&mut nbt).await;

            let removed = path.remove(&mut nbt);
            if !removed {
                return Err(CommandError::CommandFailed(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_MERGE_FAILED,
                    translation::java::COMMANDS_DATA_MERGE_FAILED,
                    [],
                )));
            }

            if let Some(new_block_entity) = block_entity_from_nbt(&nbt) {
                world.add_block_entity(new_block_entity);
            } else {
                world.add_block_entity_nbt(pos, &nbt);
            }

            sender
                .send_message(TextComponent::translate_cross(
                    translation::java::COMMANDS_DATA_BLOCK_MODIFIED,
                    translation::java::COMMANDS_DATA_BLOCK_MODIFIED,
                    [
                        TextComponent::text(pos.0.x.to_string()),
                        TextComponent::text(pos.0.y.to_string()),
                        TextComponent::text(pos.0.z.to_string()),
                    ],
                ))
                .await;

            Ok(1)
        })
    }
}

pub fn init_command_tree() -> CommandTree {
    CommandTree::new(NAMES, DESCRIPTION)
        .then(
            literal("get")
                .then(
                    literal("entity").then(
                        argument(ARG_ENTITY, EntityArgumentConsumer)
                            .execute(GetEntityDataExecutor)
                            .then(
                                argument(ARG_PATH, NbtPathArgumentConsumer)
                                    .execute(GetEntityPathDataExecutor)
                                    .then(
                                        argument(ARG_SCALE, scale_consumer())
                                            .execute(GetEntityPathScaleDataExecutor),
                                    ),
                            ),
                    ),
                )
                .then(
                    literal("block").then(
                        argument(ARG_BLOCK_POS, BlockPosArgumentConsumer)
                            .execute(GetBlockDataExecutor)
                            .then(
                                argument(ARG_PATH, NbtPathArgumentConsumer)
                                    .execute(GetBlockPathDataExecutor)
                                    .then(
                                        argument(ARG_SCALE, scale_consumer())
                                            .execute(GetBlockPathScaleDataExecutor),
                                    ),
                            ),
                    ),
                ),
        )
        .then(
            literal("merge")
                .then(
                    literal("entity").then(
                        argument(ARG_ENTITY, EntityArgumentConsumer).then(
                            argument(ARG_NBT, NbtCompoundArgumentConsumer)
                                .execute(MergeEntityDataExecutor),
                        ),
                    ),
                )
                .then(
                    literal("block").then(
                        argument(ARG_BLOCK_POS, BlockPosArgumentConsumer).then(
                            argument(ARG_NBT, NbtCompoundArgumentConsumer)
                                .execute(MergeBlockDataExecutor),
                        ),
                    ),
                ),
        )
        .then(
            literal("remove")
                .then(
                    literal("entity").then(
                        argument(ARG_ENTITY, EntityArgumentConsumer).then(
                            argument(ARG_PATH, NbtPathArgumentConsumer)
                                .execute(RemoveEntityDataExecutor),
                        ),
                    ),
                )
                .then(
                    literal("block").then(
                        argument(ARG_BLOCK_POS, BlockPosArgumentConsumer).then(
                            argument(ARG_PATH, NbtPathArgumentConsumer)
                                .execute(RemoveBlockDataExecutor),
                        ),
                    ),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_nbt::tag::NbtTag;

    #[test]
    fn test_merge_nbt_compound_scalar_overwrite_and_insert() {
        let mut target = NbtCompound::new();
        target.put("a", NbtTag::Int(1));
        target.put("b", NbtTag::String("original".into()));

        let mut incoming = NbtCompound::new();
        incoming.put("b", NbtTag::String("updated".into()));
        incoming.put("c", NbtTag::Byte(42));

        merge_nbt_compound(&mut target, &incoming);

        assert_eq!(target.get("a"), Some(&NbtTag::Int(1)));
        assert_eq!(target.get("b"), Some(&NbtTag::String("updated".into())));
        assert_eq!(target.get("c"), Some(&NbtTag::Byte(42)));
    }

    #[test]
    fn test_merge_nbt_compound_recursive() {
        let mut target_inner = NbtCompound::new();
        target_inner.put("x", NbtTag::Int(10));
        target_inner.put("y", NbtTag::Int(20));

        let mut target = NbtCompound::new();
        target.put("inner", NbtTag::Compound(target_inner));

        let mut incoming_inner = NbtCompound::new();
        incoming_inner.put("y", NbtTag::Int(99));
        incoming_inner.put("z", NbtTag::Int(30));

        let mut incoming = NbtCompound::new();
        incoming.put("inner", NbtTag::Compound(incoming_inner));

        merge_nbt_compound(&mut target, &incoming);

        let Some(NbtTag::Compound(merged_inner)) = target.get("inner") else {
            panic!("Expected compound");
        };

        assert_eq!(merged_inner.get("x"), Some(&NbtTag::Int(10)));
        assert_eq!(merged_inner.get("y"), Some(&NbtTag::Int(99)));
        assert_eq!(merged_inner.get("z"), Some(&NbtTag::Int(30)));
    }

    #[test]
    fn test_get_i32_result() {
        assert_eq!(get_i32_result(&NbtTag::Int(123)).unwrap(), 123);
        assert_eq!(get_i32_result(&NbtTag::Byte(5)).unwrap(), 5);
        assert_eq!(get_i32_result(&NbtTag::Short(10)).unwrap(), 10);
        assert_eq!(get_i32_result(&NbtTag::String("hello".into())).unwrap(), 5);
    }

    #[test]
    fn test_nbt_path_parse_and_query() {
        let path = NbtPath::parse("foo.bar[1].baz").unwrap();
        assert_eq!(
            path.segments,
            vec![
                NbtPathSegment::Key("foo".into()),
                NbtPathSegment::Key("bar".into()),
                NbtPathSegment::Index(1),
                NbtPathSegment::Key("baz".into()),
            ]
        );

        let mut item0 = NbtCompound::new();
        item0.put("baz", NbtTag::Int(11));
        let mut item1 = NbtCompound::new();
        item1.put("baz", NbtTag::Int(22));

        let list = vec![NbtTag::Compound(item0), NbtTag::Compound(item1)];

        let mut bar = NbtCompound::new();
        bar.put("bar", NbtTag::List(list));

        let mut foo = NbtCompound::new();
        foo.put("foo", NbtTag::Compound(bar));

        let root = NbtTag::Compound(foo);
        let queried = path.query(&root).unwrap();
        assert_eq!(queried, &NbtTag::Int(22));
    }

    #[test]
    fn test_nbt_path_remove() {
        let mut inner = NbtCompound::new();
        inner.put("drop", NbtTag::Int(99));
        inner.put("keep", NbtTag::String("stay".into()));

        let mut root = NbtCompound::new();
        root.put("sub", NbtTag::Compound(inner));

        let path = NbtPath::parse("sub.drop").unwrap();
        let removed = path.remove(&mut root);
        assert!(removed);

        let Some(NbtTag::Compound(sub)) = root.get("sub") else {
            panic!("Expected compound");
        };
        assert_eq!(sub.get("drop"), None);
        assert_eq!(sub.get("keep"), Some(&NbtTag::String("stay".into())));
    }
}
