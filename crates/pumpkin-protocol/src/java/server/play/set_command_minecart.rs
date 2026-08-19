use crate::{
    ServerPacket,
    codec::var_int::VarInt,
    ser::{NetworkReadExt, NetworkReadSliceExt},
};
use pumpkin_data::packet::serverbound::PLAY_SET_COMMAND_MINECART;
use pumpkin_macros::java_packet;
use pumpkin_util::version::JavaMinecraftVersion;

/// Java's command-minecart editor packet.  The entity id is deliberately a
/// VarInt (not a block position): the client sends this after interacting with
/// a `MinecartCommandBlock` entity and the server validates its subtype and
/// permission before mutating state.
#[java_packet(PLAY_SET_COMMAND_MINECART)]
pub struct SSetCommandMinecart<'a> {
    pub entity_id: VarInt,
    pub command: &'a str,
    pub track_output: bool,
}

impl<'a> ServerPacket<'a> for SSetCommandMinecart<'a> {
    fn read(
        bytebuf: &mut &'a [u8],
        _version: &JavaMinecraftVersion,
    ) -> Result<Self, crate::ser::ReadingError> {
        Ok(Self {
            entity_id: bytebuf.get_var_int()?,
            command: bytebuf.get_str_bounded_borrowed(32767)?,
            track_output: bytebuf.get_bool()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SSetCommandMinecart;
    use crate::ServerPacket;
    use pumpkin_util::version::JavaMinecraftVersion;

    #[test]
    fn command_minecart_packet_decodes_entity_command_and_track_flag() {
        let mut bytes = &[42, 7, b's', b'a', b'y', b' ', b'h', b'i', b'\n', 1][..];
        let packet = SSetCommandMinecart::read(&mut bytes, &JavaMinecraftVersion::V_26_2)
            .expect("packet must decode");
        assert_eq!(packet.entity_id.0, 42);
        assert_eq!(packet.command, "say hi\n");
        assert!(packet.track_output);
        assert!(bytes.is_empty());
    }
}
