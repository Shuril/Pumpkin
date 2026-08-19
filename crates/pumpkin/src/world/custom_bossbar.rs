use crate::command::args::GetCloned;
use crate::entity::player::Player;
use crate::server::Server;
use crate::world::bossbar::BossbarFlags;
use crate::world::bossbar::{Bossbar, BossbarColor, BossbarDivisions};
use pumpkin_nbt::{
    compound::NbtCompound,
    nbt_compress::{read_gzip_compound_tag, write_gzip_compound_tag},
    tag::NbtTag,
};
use pumpkin_util::text::TextComponent;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum BossbarUpdateError {
    #[error("Invalid resource location")]
    InvalidResourceLocation(String),
    #[error("No changes")]
    NoChanges(&'static str, Option<&'static str>),
}

fn parse_bossbar_color(value: Option<&str>) -> BossbarColor {
    match value {
        Some("pink") => BossbarColor::Pink,
        Some("blue") => BossbarColor::Blue,
        Some("red") => BossbarColor::Red,
        Some("green") => BossbarColor::Green,
        Some("yellow") => BossbarColor::Yellow,
        Some("purple") => BossbarColor::Purple,
        _ => BossbarColor::White,
    }
}

const fn bossbar_color_name(color: BossbarColor) -> &'static str {
    match color {
        BossbarColor::Pink => "pink",
        BossbarColor::Blue => "blue",
        BossbarColor::Red => "red",
        BossbarColor::Green => "green",
        BossbarColor::Yellow => "yellow",
        BossbarColor::Purple => "purple",
        BossbarColor::White => "white",
    }
}

fn parse_bossbar_division(value: Option<&str>) -> BossbarDivisions {
    match value {
        Some("notches_6") => BossbarDivisions::Notches6,
        Some("notches_10") => BossbarDivisions::Notches10,
        Some("notches_12") => BossbarDivisions::Notches12,
        Some("notches_20") => BossbarDivisions::Notches20,
        _ => BossbarDivisions::NoDivision,
    }
}

const fn bossbar_division_name(division: BossbarDivisions) -> &'static str {
    match division {
        BossbarDivisions::NoDivision => "progress",
        BossbarDivisions::Notches6 => "notches_6",
        BossbarDivisions::Notches10 => "notches_10",
        BossbarDivisions::Notches12 => "notches_12",
        BossbarDivisions::Notches20 => "notches_20",
    }
}

/// Representing the stored custom boss bars from level.dat
#[derive(Clone)]
pub struct CustomBossbar {
    pub namespace: String,
    pub bossbar_data: Bossbar,
    pub max: i32,
    pub value: i32,
    pub visible: bool,
    pub players: Vec<Uuid>,
}

impl CustomBossbar {
    #[deny(clippy::new_without_default)]
    #[must_use]
    pub const fn new(namespace: String, bossbar_data: Bossbar) -> Self {
        Self {
            namespace,
            bossbar_data,
            max: 100,
            value: 0,
            visible: true,
            players: vec![],
        }
    }
}

pub struct CustomBossbars {
    pub custom_bossbars: HashMap<String, CustomBossbar>,
    /// Entries from the data file which this server cannot interpret yet.
    /// They are copied back verbatim on save instead of being discarded.
    unknown_bossbars: NbtCompound,
    load_complete: bool,
}

impl Default for CustomBossbars {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomBossbars {
    #[must_use]
    pub fn new() -> Self {
        Self {
            custom_bossbars: HashMap::new(),
            unknown_bossbars: NbtCompound::new(),
            load_complete: false,
        }
    }

    /// Loads the vanilla `data/minecraft/custom_boss_events.dat` file.
    ///
    /// Vanilla stores the resource location as the key under `data`; the
    /// UUID is intentionally not persisted (a new runtime UUID is assigned
    /// on every load). Unknown entries are ignored by the live manager but
    /// retained by [`save_to_world_path`] because it starts from the existing
    /// NBT root.
    pub fn load_from_world_path(&mut self, level_folder: &Path) -> Result<(), String> {
        let path = level_folder
            .join("data")
            .join("minecraft")
            .join("custom_boss_events.dat");
        let root = match File::open(&path) {
            Ok(file) => read_gzip_compound_tag(file).map_err(|e| e.to_string())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.load_complete = true;
                return Ok(());
            }
            Err(error) => return Err(error.to_string()),
        };
        let Some(data) = root.get_compound("data") else {
            self.load_complete = true;
            return Ok(());
        };
        self.custom_bossbars.clear();
        self.unknown_bossbars = data.clone();
        self.unknown_bossbars.child_tags.remove("DataVersion");

        for (namespace, tag) in &data.child_tags {
            if namespace.as_ref() == "DataVersion" {
                continue;
            }
            let NbtTag::Compound(event) = tag else {
                continue;
            };
            let Some(name) = event.get_string("Name") else {
                continue;
            };
            let title = serde_json::from_str::<TextComponent>(name)
                .unwrap_or_else(|_| TextComponent::text(name.to_owned()));
            let mut bossbar = Bossbar::new(title);
            bossbar.color = parse_bossbar_color(event.get_string("Color"));
            bossbar.division = parse_bossbar_division(event.get_string("Overlay"));
            let darken = event.get_bool("DarkenScreen").unwrap_or(false);
            let music = event.get_bool("PlayBossMusic").unwrap_or(false);
            let fog = event.get_bool("CreateWorldFog").unwrap_or(false);
            bossbar.flags = BossbarFlags::from_bits_retain(
                (if darken {
                    BossbarFlags::DARKEN_SKY.bits()
                } else {
                    0
                }) | (if music {
                    BossbarFlags::DRAGON_BAR.bits()
                } else {
                    0
                }) | (if fog {
                    BossbarFlags::CREATE_FOG.bits()
                } else {
                    0
                }),
            );
            let max = event.get_int("Max").unwrap_or(100).max(1);
            let value = event.get_int("Value").unwrap_or(0).clamp(0, max);
            bossbar.health = value as f32 / max as f32;
            let players = event
                .get_list("Players")
                .unwrap_or(&[])
                .iter()
                .filter_map(|tag| match tag {
                    NbtTag::IntArray(values) if values.len() == 4 => Some(Uuid::from_u128(
                        ((values[0] as u32 as u128) << 96)
                            | ((values[1] as u32 as u128) << 64)
                            | ((values[2] as u32 as u128) << 32)
                            | (values[3] as u32 as u128),
                    )),
                    _ => None,
                })
                .collect();
            self.custom_bossbars.insert(
                namespace.to_string(),
                CustomBossbar {
                    namespace: namespace.to_string(),
                    bossbar_data: bossbar,
                    max,
                    value,
                    visible: event.get_bool("Visible").unwrap_or(false),
                    players,
                },
            );
            self.unknown_bossbars.child_tags.remove(namespace.as_ref());
        }
        self.load_complete = true;
        Ok(())
    }

    /// Writes the live manager into the vanilla custom boss-events file.
    /// Existing root/data entries are preserved so newer vanilla fields and
    /// plugin-owned bars are not silently deleted.
    pub fn save_to_world_path(&self, level_folder: &Path, data_version: i32) -> Result<(), String> {
        let dir = level_folder.join("data").join("minecraft");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("custom_boss_events.dat");
        if !self.load_complete && path.exists() {
            return Err(
                "refusing to overwrite custom_boss_events.dat because it was not loaded successfully"
                    .to_owned(),
            );
        }
        let mut root = match File::open(&path) {
            Ok(file) => read_gzip_compound_tag(file).map_err(|e| e.to_string())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => NbtCompound::new(),
            Err(error) => return Err(error.to_string()),
        };
        let mut data = self.unknown_bossbars.clone();
        data.put_int("DataVersion", data_version);
        for (namespace, bossbar) in &self.custom_bossbars {
            let mut event = NbtCompound::new();
            let title =
                serde_json::to_string(&bossbar.bossbar_data.title).map_err(|e| e.to_string())?;
            event.put_string("Name", title);
            event.put_bool("Visible", bossbar.visible);
            event.put_int("Value", bossbar.value);
            event.put_int("Max", bossbar.max.max(1));
            event.put_string(
                "Color",
                bossbar_color_name(bossbar.bossbar_data.color).to_owned(),
            );
            event.put_string(
                "Overlay",
                bossbar_division_name(bossbar.bossbar_data.division).to_owned(),
            );
            event.put_bool(
                "DarkenScreen",
                bossbar
                    .bossbar_data
                    .flags
                    .contains(BossbarFlags::DARKEN_SKY),
            );
            event.put_bool(
                "PlayBossMusic",
                bossbar
                    .bossbar_data
                    .flags
                    .contains(BossbarFlags::DRAGON_BAR),
            );
            event.put_bool(
                "CreateWorldFog",
                bossbar
                    .bossbar_data
                    .flags
                    .contains(BossbarFlags::CREATE_FOG),
            );
            let players = bossbar
                .players
                .iter()
                .map(|uuid| {
                    let value = uuid.as_u128();
                    NbtTag::IntArray(vec![
                        (value >> 96) as i32,
                        ((value >> 64) & 0xFFFF_FFFF) as i32,
                        ((value >> 32) & 0xFFFF_FFFF) as i32,
                        (value & 0xFFFF_FFFF) as i32,
                    ])
                })
                .collect();
            event.put_list("Players", players);
            data.put_compound(namespace, event);
        }
        root.put_compound("data", data);
        write_gzip_compound_tag(
            root,
            BufWriter::new(File::create(path).map_err(|e| e.to_string())?),
        )
        .map_err(|e| e.to_string())
    }

    #[must_use]
    pub fn get_player_bars(&self, uuid: &Uuid) -> Option<Vec<&Bossbar>> {
        let mut player_bars: Vec<&Bossbar> = Vec::new();
        for bossbar in &self.custom_bossbars {
            if bossbar.1.players.contains(uuid) {
                player_bars.push(&bossbar.1.bossbar_data);
            }
        }
        if !player_bars.is_empty() {
            return Some(player_bars);
        }
        None
    }

    pub fn create_bossbar(&mut self, namespace: String, bossbar_data: Bossbar) {
        self.unknown_bossbars.child_tags.remove(namespace.as_str());
        self.custom_bossbars.insert(
            namespace.clone(),
            CustomBossbar::new(namespace, bossbar_data),
        );
    }

    pub fn replace_bossbar(&mut self, resource_location: String, bossbar_data: CustomBossbar) {
        self.unknown_bossbars
            .child_tags
            .remove(resource_location.as_str());
        self.custom_bossbars.insert(resource_location, bossbar_data);
    }

    #[must_use]
    pub fn get_all_bossbars(&self) -> Vec<CustomBossbar> {
        let mut bossbars: Vec<CustomBossbar> = Vec::new();
        for bossbar in self.custom_bossbars.clone() {
            bossbars.push(bossbar.1);
        }
        bossbars
    }

    #[must_use]
    pub fn get_bossbars_len(&self) -> usize {
        self.custom_bossbars.len()
    }

    #[must_use]
    pub fn get_bossbar(&self, resource_location: &str) -> Option<CustomBossbar> {
        let bossbar = self.custom_bossbars.get(resource_location);
        if let Some(bossbar) = bossbar {
            return Some(bossbar.clone());
        }
        None
    }

    pub async fn remove_bossbar(
        &mut self,
        server: &Server,
        resource_location: String,
    ) -> Result<(), BossbarUpdateError> {
        let bossbar = self.custom_bossbars.get_cloned(&resource_location);
        if let Some(bossbar) = bossbar {
            self.custom_bossbars.remove(&resource_location);

            let players: Vec<Arc<Player>> = server.get_all_players();

            let online_players = players
                .iter()
                .filter(|player| bossbar.players.contains(&player.gameprofile.id));

            if bossbar.visible {
                for player in online_players {
                    player.remove_bossbar(bossbar.bossbar_data.uuid).await;
                }
            }

            return Ok(());
        }
        Err(BossbarUpdateError::InvalidResourceLocation(
            resource_location,
        ))
    }

    #[must_use]
    pub fn has_bossbar(&self, resource_location: &str) -> bool {
        self.custom_bossbars.contains_key(resource_location)
    }

    pub async fn update_health(
        &mut self,
        server: &Server,
        resource_location: String,
        max_value: i32,
        value: i32,
    ) -> Result<(), BossbarUpdateError> {
        let bossbar = self.custom_bossbars.get_mut(&resource_location);
        if let Some(bossbar) = bossbar {
            if bossbar.value == value && bossbar.max == max_value {
                return Err(BossbarUpdateError::NoChanges("value", None));
            }

            let ratio = f64::from(value) / f64::from(max_value);

            let health: f32 = if ratio < 0.0 {
                0.0
            } else if ratio > 1.0 {
                1.0
            } else {
                ratio as f32
            };

            bossbar.value = value;
            bossbar.max = max_value;
            bossbar.bossbar_data.health = health;

            if !bossbar.visible {
                return Ok(());
            }

            let players: Vec<Arc<Player>> = server.get_all_players();
            let matching_players = players
                .iter()
                .filter(|player| bossbar.players.contains(&player.gameprofile.id));
            for player in matching_players {
                player
                    .update_bossbar_health(&bossbar.bossbar_data.uuid, bossbar.bossbar_data.health)
                    .await;
            }

            return Ok(());
        }
        Err(BossbarUpdateError::InvalidResourceLocation(
            resource_location,
        ))
    }

    pub async fn update_visibility(
        &mut self,
        server: &Server,
        resource_location: String,
        new_visibility: bool,
    ) -> Result<(), BossbarUpdateError> {
        let bossbar = self.custom_bossbars.get_mut(&resource_location);
        if let Some(bossbar) = bossbar {
            if bossbar.visible == new_visibility && new_visibility {
                return Err(BossbarUpdateError::NoChanges("visibility", Some("visible")));
            }

            if bossbar.visible == new_visibility && !new_visibility {
                return Err(BossbarUpdateError::NoChanges("visibility", Some("hidden")));
            }

            bossbar.visible = new_visibility;

            let players: Vec<Arc<Player>> = server.get_all_players();
            let online_players = players
                .iter()
                .filter(|player| bossbar.players.contains(&player.gameprofile.id));

            for player in online_players {
                if bossbar.visible {
                    player.send_bossbar(&bossbar.bossbar_data).await;
                } else {
                    player.remove_bossbar(bossbar.bossbar_data.uuid).await;
                }
            }

            return Ok(());
        }
        Err(BossbarUpdateError::InvalidResourceLocation(
            resource_location,
        ))
    }

    pub async fn update_name(
        &mut self,
        server: &Server,
        resource_location: &str,
        new_title: TextComponent,
    ) -> Result<(), BossbarUpdateError> {
        let bossbar = self.custom_bossbars.get_mut(resource_location);
        if let Some(bossbar) = bossbar {
            if bossbar.bossbar_data.title == new_title {
                return Err(BossbarUpdateError::NoChanges("name", None));
            }

            bossbar.bossbar_data.title = new_title;

            if !bossbar.visible {
                return Ok(());
            }

            let players: Vec<Arc<Player>> = server.get_all_players();
            let matching_players = players
                .iter()
                .filter(|player| bossbar.players.contains(&player.gameprofile.id));
            for player in matching_players {
                player
                    .update_bossbar_title(
                        &bossbar.bossbar_data.uuid,
                        bossbar.bossbar_data.title.clone(),
                    )
                    .await;
            }

            return Ok(());
        }
        Err(BossbarUpdateError::InvalidResourceLocation(
            resource_location.to_string(),
        ))
    }

    pub async fn update_color(
        &mut self,
        server: &Server,
        resource_location: String,
        new_color: BossbarColor,
    ) -> Result<(), BossbarUpdateError> {
        let bossbar = self.custom_bossbars.get_mut(&resource_location);
        if let Some(bossbar) = bossbar {
            if bossbar.bossbar_data.color == new_color {
                return Err(BossbarUpdateError::NoChanges("color", None));
            }

            bossbar.bossbar_data.color = new_color;

            if !bossbar.visible {
                return Ok(());
            }

            let players: Vec<Arc<Player>> = server.get_all_players();
            let matching_players = players
                .iter()
                .filter(|player| bossbar.players.contains(&player.gameprofile.id));
            for player in matching_players {
                player
                    .update_bossbar_style(
                        &bossbar.bossbar_data.uuid,
                        bossbar.bossbar_data.color,
                        bossbar.bossbar_data.division,
                        bossbar.bossbar_data.flags,
                    )
                    .await;
            }

            return Ok(());
        }
        Err(BossbarUpdateError::InvalidResourceLocation(
            resource_location,
        ))
    }

    pub async fn update_division(
        &mut self,
        server: &Server,
        resource_location: String,
        new_division: BossbarDivisions,
    ) -> Result<(), BossbarUpdateError> {
        let bossbar = self.custom_bossbars.get_mut(&resource_location);
        if let Some(bossbar) = bossbar {
            if bossbar.bossbar_data.division == new_division {
                return Err(BossbarUpdateError::NoChanges("style", None));
            }

            bossbar.bossbar_data.division = new_division;

            if !bossbar.visible {
                return Ok(());
            }

            let players: Vec<Arc<Player>> = server.get_all_players();
            let matching_players = players
                .iter()
                .filter(|player| bossbar.players.contains(&player.gameprofile.id));
            for player in matching_players {
                player
                    .update_bossbar_style(
                        &bossbar.bossbar_data.uuid,
                        bossbar.bossbar_data.color,
                        bossbar.bossbar_data.division,
                        bossbar.bossbar_data.flags,
                    )
                    .await;
            }

            return Ok(());
        }
        Err(BossbarUpdateError::InvalidResourceLocation(
            resource_location,
        ))
    }

    pub async fn update_players(
        &mut self,
        server: &Server,
        resource_location: String,
        new_players: Vec<Uuid>,
    ) -> Result<(), BossbarUpdateError> {
        let bossbar = self.custom_bossbars.get_mut(&resource_location);
        if let Some(bossbar) = bossbar {
            // Get the difference between the old and new player list and remove bossbars from old players.
            let removed_players: Vec<Uuid> = bossbar
                .players
                .iter()
                .filter(|item| !new_players.contains(item))
                .copied()
                .collect();

            let added_players: Vec<Uuid> = new_players
                .iter()
                .filter(|item| !bossbar.players.contains(item))
                .copied()
                .collect();

            if removed_players.is_empty() && added_players.is_empty() {
                return Err(BossbarUpdateError::NoChanges("players", None));
            }

            if bossbar.visible {
                for uuid in removed_players {
                    let Some(player) = server.get_player_by_uuid(uuid) else {
                        continue;
                    };

                    player.remove_bossbar(bossbar.bossbar_data.uuid).await;
                }
            }

            bossbar.players = new_players;

            if !bossbar.visible {
                return Ok(());
            }

            for uuid in added_players {
                let Some(player) = server.get_player_by_uuid(uuid) else {
                    continue;
                };

                player.send_bossbar(&bossbar.bossbar_data).await;
            }

            return Ok(());
        }
        Err(BossbarUpdateError::InvalidResourceLocation(
            resource_location,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{BossbarColor, BossbarDivisions, CustomBossbars};
    use crate::world::bossbar::{Bossbar, BossbarFlags};
    use pumpkin_nbt::{
        compound::NbtCompound,
        nbt_compress::{read_gzip_compound_tag, write_gzip_compound_tag},
    };
    use pumpkin_util::text::TextComponent;
    use std::fs::{self, File};
    use tempfile::tempdir;
    use uuid::Uuid;

    #[test]
    fn custom_bossbars_round_trip_vanilla_data_file() {
        let directory = tempdir().unwrap();
        let data_dir = directory.path().join("data/minecraft");
        fs::create_dir_all(&data_dir).unwrap();
        let mut root = NbtCompound::new();
        root.put_string("FutureRoot", "kept".to_owned());
        let mut data = NbtCompound::new();
        let mut future = NbtCompound::new();
        future.put_int("FutureField", 7);
        data.put_compound("future:bar", future);
        root.put_compound("data", data);
        write_gzip_compound_tag(
            root,
            File::create(data_dir.join("custom_boss_events.dat")).unwrap(),
        )
        .unwrap();

        let mut bars = CustomBossbars::new();
        bars.load_from_world_path(directory.path()).unwrap();
        let mut bossbar = Bossbar::new(TextComponent::text("Dragon"));
        bossbar.color = BossbarColor::Red;
        bossbar.division = BossbarDivisions::Notches10;
        bossbar.flags = BossbarFlags::DARKEN_SKY | BossbarFlags::DRAGON_BAR;
        bars.create_bossbar("minecraft:dragon".to_owned(), bossbar);
        let player = Uuid::from_u128(1);
        let stored = bars.custom_bossbars.get_mut("minecraft:dragon").unwrap();
        stored.max = 200;
        stored.value = 125;
        stored.visible = false;
        stored.players.push(player);
        bars.save_to_world_path(directory.path(), 4903).unwrap();

        let mut loaded = CustomBossbars::new();
        loaded.load_from_world_path(directory.path()).unwrap();
        let loaded_bar = loaded.get_bossbar("minecraft:dragon").unwrap();
        assert_eq!(loaded_bar.max, 200);
        assert_eq!(loaded_bar.value, 125);
        assert!(!loaded_bar.visible);
        assert_eq!(loaded_bar.players, vec![player]);
        assert!(matches!(loaded_bar.bossbar_data.color, BossbarColor::Red));
        assert!(matches!(
            loaded_bar.bossbar_data.division,
            BossbarDivisions::Notches10
        ));
        assert!(
            loaded_bar
                .bossbar_data
                .flags
                .contains(BossbarFlags::DARKEN_SKY)
        );

        let rewritten =
            read_gzip_compound_tag(File::open(data_dir.join("custom_boss_events.dat")).unwrap())
                .unwrap();
        assert_eq!(rewritten.get_string("FutureRoot"), Some("kept"));
        assert_eq!(
            rewritten
                .get_compound("data")
                .and_then(|data| data.get_compound("future:bar"))
                .and_then(|data| data.get_int("FutureField")),
            Some(7)
        );
        assert_eq!(
            rewritten
                .get_compound("data")
                .and_then(|data| data.get_int("DataVersion")),
            Some(4903)
        );
    }
}
