/// Size of the fixed GameUpdatePacket header.
pub const HEADER_SIZE: usize = 56;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GamePacketType {
    #[default]
    State,
    CallFunction,
    UpdateStatus,
    TileChangeRequest,
    SendMapData,
    SendTileUpdateData,
    SendTileUpdateDataMultiple,
    TileActivateRequest,
    TileApplyDamage,
    SendInventoryState,
    ItemActivateRequest,
    ItemActivateObjectRequest,
    SendTileTreeState,
    ModifyItemInventory,
    ItemChangeObject,
    SendLock,
    SendItemDatabaseData,
    SendParticleEffect,
    SetIconState,
    ItemEffect,
    SetCharacterState,
    PingReply,
    PingRequest,
    GotPunched,
    AppCheckResponse,
    AppIntegrityFail,
    Disconnect,
    BattleJoin,
    BattleEvent,
    UseDoor,
    SendParental,
    GoneFishin,
    Steam,
    PetBattle,
    Npc,
    Special,
    SendParticleEffectV2,
    ActiveArrowToItem,
    SelectTileIndex,
    SendPlayerTributeData,
    FtueSetItemToQuickInventory,
    PveNpc,
    PvpCardBattle,
    PveApplyPlayerDamage,
    PveNpcPositionUpdate,
    SetExtraMods,
    OnStepOnTileMod,
    Unknown(u8),
}

/// Ordered so that `KNOWN[i]` has wire value `i`.
const KNOWN: [GamePacketType; 47] = {
    use GamePacketType::*;
    [
        State, CallFunction, UpdateStatus, TileChangeRequest, SendMapData, SendTileUpdateData,
        SendTileUpdateDataMultiple, TileActivateRequest, TileApplyDamage, SendInventoryState,
        ItemActivateRequest, ItemActivateObjectRequest, SendTileTreeState, ModifyItemInventory,
        ItemChangeObject, SendLock, SendItemDatabaseData, SendParticleEffect, SetIconState,
        ItemEffect, SetCharacterState, PingReply, PingRequest, GotPunched, AppCheckResponse,
        AppIntegrityFail, Disconnect, BattleJoin, BattleEvent, UseDoor, SendParental, GoneFishin,
        Steam, PetBattle, Npc, Special, SendParticleEffectV2, ActiveArrowToItem, SelectTileIndex,
        SendPlayerTributeData, FtueSetItemToQuickInventory, PveNpc, PvpCardBattle,
        PveApplyPlayerDamage, PveNpcPositionUpdate, SetExtraMods, OnStepOnTileMod,
    ]
};

impl GamePacketType {
    pub fn from_u8(v: u8) -> Self {
        KNOWN.get(v as usize).copied().unwrap_or(Self::Unknown(v))
    }

    pub fn as_u8(self) -> u8 {
        match self {
            Self::Unknown(v) => v,
            known => KNOWN.iter().position(|&k| k == known).unwrap() as u8,
        }
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct PacketFlags: u32 {
        const WALK          = 0x0000_0001;
        const SPAWN_RELATED = 0x0000_0004;
        /// Extra data follows the header.
        const EXTENDED      = 0x0000_0008;
        const FACING_LEFT   = 0x0000_0010;
        const STANDING      = 0x0000_0020;
        const FIRE_DAMAGE   = 0x0000_0040;
        const JUMP          = 0x0000_0080;
        const GOT_KILLED    = 0x0000_0100;
        const PUNCH         = 0x0000_0200;
        const PLACE         = 0x0000_0400;
        const TILE_CHANGE   = 0x0000_0800;
        const GOT_PUNCHED   = 0x0000_1000;
        const RESPAWN       = 0x0000_2000;
        const OBJECT_COLLECT= 0x0000_4000;
        const TRAMPOLINE    = 0x0000_8000;
        const DAMAGE        = 0x0001_0000;
        const SLIDE         = 0x0002_0000;
        const PARASOL       = 0x0004_0000;
        const SWIM          = 0x0010_0000;
        const WALL_HANG     = 0x0020_0000;
        const ACID_DAMAGE   = 0x1000_0000;
    }
}

/// The 56-byte game packet (`NET_MESSAGE_GAME_PACKET`, type 4).
#[derive(Debug, Clone, Default)]
pub struct GameUpdatePacket {
    pub packet_type: GamePacketType,
    pub object_type: u8,
    pub jump_count: u8,
    pub animation_type: u8,
    pub net_id: i32,
    pub target_net_id: i32,
    pub flags: PacketFlags,
    pub float_var: f32,
    pub value: u32,
    pub vec_x: f32,
    pub vec_y: f32,
    pub vec_x2: f32,
    pub vec_y2: f32,
    pub particle_rotation: f32,
    pub int_x: i32,
    pub int_y: i32,
    pub extra: Vec<u8>,
}

impl GameUpdatePacket {
    /// Parses the body after the 4-byte message type.
    pub fn from_bytes(d: &[u8]) -> Option<Self> {
        if d.len() < HEADER_SIZE {
            return None;
        }
        let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let i32_at = |o: usize| u32_at(o) as i32;
        let f32_at = |o: usize| f32::from_bits(u32_at(o));

        let flags = PacketFlags::from_bits_retain(u32_at(12));
        let extra_len = u32_at(52) as usize;
        let extra = if flags.contains(PacketFlags::EXTENDED) {
            d.get(HEADER_SIZE..HEADER_SIZE.checked_add(extra_len)?)?.to_vec()
        } else {
            Vec::new()
        };

        Some(Self {
            packet_type: GamePacketType::from_u8(d[0]),
            object_type: d[1],
            jump_count: d[2],
            animation_type: d[3],
            net_id: i32_at(4),
            target_net_id: i32_at(8),
            flags,
            float_var: f32_at(16),
            value: u32_at(20),
            vec_x: f32_at(24),
            vec_y: f32_at(28),
            vec_x2: f32_at(32),
            vec_y2: f32_at(36),
            particle_rotation: f32_at(40),
            int_x: i32_at(44),
            int_y: i32_at(48),
            extra,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let extended = self.flags.contains(PacketFlags::EXTENDED);
        let extra_len = if extended { self.extra.len() } else { 0 };
        let mut b = Vec::with_capacity(HEADER_SIZE + extra_len);
        b.extend_from_slice(&[self.packet_type.as_u8(), self.object_type, self.jump_count, self.animation_type]);
        for v in [
            self.net_id as u32,
            self.target_net_id as u32,
            self.flags.bits(),
            self.float_var.to_bits(),
            self.value,
            self.vec_x.to_bits(),
            self.vec_y.to_bits(),
            self.vec_x2.to_bits(),
            self.vec_y2.to_bits(),
            self.particle_rotation.to_bits(),
            self.int_x as u32,
            self.int_y as u32,
            extra_len as u32,
        ] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        if extended {
            b.extend_from_slice(&self.extra);
        }
        b
    }

    /// A `CallFunction` packet carrying a serialized VariantList.
    pub fn call_function(net_id: i32, variants: Vec<u8>) -> Self {
        Self {
            packet_type: GamePacketType::CallFunction,
            net_id,
            flags: PacketFlags::EXTENDED,
            extra: variants,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_mapping() {
        for v in 0..=255u8 {
            assert_eq!(GamePacketType::from_u8(v).as_u8(), v);
        }
        assert_eq!(GamePacketType::from_u8(1), GamePacketType::CallFunction);
        assert_eq!(GamePacketType::from_u8(46), GamePacketType::OnStepOnTileMod);
    }

    #[test]
    fn roundtrip() {
        let pkt = GameUpdatePacket {
            packet_type: GamePacketType::TileChangeRequest,
            net_id: -1,
            int_x: 18,
            int_y: 8,
            vec_x: 576.0,
            flags: PacketFlags::EXTENDED | PacketFlags::PUNCH,
            extra: vec![1, 2, 3],
            ..Default::default()
        };
        let bytes = pkt.to_bytes();
        assert_eq!(bytes.len(), HEADER_SIZE + 3);
        let back = GameUpdatePacket::from_bytes(&bytes).unwrap();
        assert_eq!(back.to_bytes(), bytes);
        assert_eq!(back.int_x, 18);
        assert_eq!(back.extra, vec![1, 2, 3]);
    }
}
