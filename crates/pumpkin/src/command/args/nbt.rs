//! SNBT compound argument used by commands such as `/summon`.

use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::java::client::play::{ArgumentType, SuggestionProviders};

use crate::{
    command::{
        CommandSender,
        args::{
            Arg, ArgumentConsumer, ConsumeResult, DefaultNameArgConsumer, FindArg,
            GetClientSideArgParser,
        },
        dispatcher::CommandError,
        snbt::SnbtParser,
        string_reader::StringReader,
        tree::RawArgs,
    },
    server::Server,
};

/// Parses one compound SNBT value.  `CommandDispatcher::split_parts` keeps
/// braces together, including embedded spaces, so this consumer receives the
/// complete compound as one raw argument.
pub struct NbtCompoundArgumentConsumer;

impl GetClientSideArgParser for NbtCompoundArgumentConsumer {
    fn get_client_side_parser(&self) -> ArgumentType {
        ArgumentType::NbtCompound
    }

    fn get_client_side_suggestion_type_override(&self) -> Option<SuggestionProviders> {
        None
    }
}

impl ArgumentConsumer for NbtCompoundArgumentConsumer {
    fn consume<'a>(
        &'a self,
        _sender: &'a CommandSender,
        _server: &'a Server,
        args: &mut RawArgs<'a>,
    ) -> ConsumeResult<'a> {
        let parsed = args.pop().and_then(|raw| {
            let mut reader = StringReader::new(raw.value);
            match SnbtParser::parse_for_commands(&mut reader) {
                Ok(NbtTag::Compound(compound)) => Some(Arg::Nbt(compound)),
                _ => None,
            }
        });
        Box::pin(async move { parsed })
    }
}

impl DefaultNameArgConsumer for NbtCompoundArgumentConsumer {
    fn default_name(&self) -> &'static str {
        "nbt"
    }
}

impl<'a> FindArg<'a> for NbtCompoundArgumentConsumer {
    type Data = NbtCompound;

    fn find_arg(args: &'a super::ConsumedArgs, name: &str) -> Result<Self::Data, CommandError> {
        match args.get(name) {
            Some(Arg::Nbt(compound)) => Ok(compound.clone()),
            _ => Err(CommandError::InvalidConsumption(Some(name.to_string()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::command::string_reader::StringReader;
    use pumpkin_nbt::tag::NbtTag;

    #[test]
    fn parses_nested_snbt_as_one_argument() {
        let input = "{Pos:[1.0d, 2.0d, 3.0d],PersistenceRequired:1b}";
        let mut reader = StringReader::new(input);
        let parsed = crate::command::snbt::SnbtParser::parse_for_commands(&mut reader).unwrap();
        assert!(matches!(parsed, NbtTag::Compound(_)));
    }
}
