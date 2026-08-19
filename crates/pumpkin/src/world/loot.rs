use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag;
use pumpkin_data::{Block, BlockState, item::Item};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::{
    loot_table::{
        LootCondition, LootFunction, LootFunctionBonusParameter, LootFunctionNumberProvider,
        LootFunctionTypes, LootPoolEntry, LootPoolEntryTypes, LootTable,
    },
    random::{RandomGenerator, RandomImpl, get_seed, xoroshiro128::Xoroshiro},
};
use serde_json::Value;
use std::sync::Arc;

use crate::world::World;

/// Derives a stable loot RNG seed for a world-bound source.  Vanilla owns one
/// world RNG stream, but Pumpkin evaluates loot in async tasks where borrowing
/// that stream would couple unrelated block/entity operations.  Hashing the
/// same observable inputs gives replay-stable output without sharing mutable
/// RNG state; the `salt` separates block, explosion, entity and command
/// sources that happen to occur at the same position and tick.
#[must_use]
pub fn derive_loot_seed(
    world_seed: u64,
    position: Option<pumpkin_util::math::vector3::Vector3<f64>>,
    world_time: u64,
    salt: u64,
) -> u64 {
    let mut value = world_seed ^ world_time.rotate_left(29) ^ salt;
    if let Some(position) = position {
        value ^= position.x.to_bits().rotate_left(7);
        value ^= position.y.to_bits().rotate_left(19);
        value ^= position.z.to_bits().rotate_left(37);
    }
    // SplitMix64 finalizer; this is a hash, not a gameplay RNG, so the exact
    // constants are intentionally fixed as part of the replay contract.
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

#[derive(Default, Clone)]
pub struct LootContextParameters {
    pub explosion_radius: Option<f32>,
    pub block_state: Option<&'static BlockState>,
    pub killed_by_player: Option<bool>,
    pub luck: f32,
    pub this_entity: Option<&'static EntityType>,
    pub killer_entity: Option<&'static EntityType>,
    pub direct_killer_entity: Option<&'static EntityType>,
    pub position: Option<pumpkin_util::math::vector3::Vector3<f64>>,
    /// Biome at the context position, using the generated registry name
    /// without a namespace (for example `plains`).
    pub biome: Option<&'static str>,
    /// Optional world-backed lookup for `LocationCheck` predicates with an
    /// offset.  Keeping the resolver in the context avoids borrowing a
    /// `World` through the loot-table data and lets deferred/async callers
    /// retain the exact lookup semantics.  Callers that cannot provide a
    /// resolver deliberately fail closed for non-zero offsets.
    pub biome_resolver: Option<Arc<dyn Fn(BlockPos) -> &'static str + Send + Sync>>,
    pub world_time: u64,
    pub damage_type: Option<DamageType>,
    pub tool: Option<ItemStack>,
    pub is_raining: Option<bool>,
    pub is_thundering: Option<bool>,
    /// Whether the killed entity was on fire at death time.
    /// Computed from `Entity.fire_ticks > 0`.
    pub is_on_fire: Option<bool>,
    /// Stable seed for every roll in this evaluation. Runtime callers should
    /// derive it with [`derive_loot_seed`], while deferred container loot uses
    /// the seed persisted next to its loot-table key. `None` is retained only
    /// for extension/test callers and uses a fresh seed.
    pub random_seed: Option<u64>,
}

impl LootContextParameters {
    /// Attach the authoritative world biome lookup unless a caller already
    /// supplied a custom resolver (for example a test or a generation view).
    pub fn attach_biome_resolver(&mut self, world: &Arc<World>) {
        if self.biome_resolver.is_some() {
            return;
        }
        let world = Arc::clone(world);
        self.biome_resolver = Some(Arc::new(move |position| {
            world.get_biome(&position).registry_id
        }));
    }
}

pub trait LootTableExt {
    fn get_loot(&self, params: LootContextParameters) -> Vec<ItemStack>;
}

impl LootTableExt for LootTable {
    fn get_loot(&self, params: LootContextParameters) -> Vec<ItemStack> {
        let mut stacks = Vec::new();
        let mut random = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(
            params.random_seed.unwrap_or_else(get_seed),
        ));

        if let Some(pools) = self.pools {
            for pool in pools {
                if let Some(conditions) = pool.conditions
                    && !conditions
                        .iter()
                        .all(|cond| cond.is_fulfilled_with_rng(&params, &mut random))
                {
                    continue;
                }

                let rolls = pool.rolls.get(&mut random) as i32
                    + (pool.bonus_rolls.get(&mut random) * params.luck).floor() as i32;

                for _ in 0..rolls {
                    let mut total_weight = 0;
                    let mut valid_entries = Vec::new();

                    for entry in pool.entries {
                        if entry.conditions.as_ref().is_none_or(|c| {
                            c.iter()
                                .all(|cond| cond.is_fulfilled_with_rng(&params, &mut random))
                        }) {
                            let weight = (entry.weight as f32 + entry.quality as f32 * params.luck)
                                .floor() as i32;
                            let weight = weight.max(0);
                            total_weight += weight;
                            valid_entries.push((entry, weight));
                        }
                    }

                    if total_weight == 0 || valid_entries.is_empty() {
                        continue;
                    }

                    let mut r = random.next_bounded_i32(total_weight);

                    for (entry, weight) in valid_entries {
                        r -= weight;
                        if r < 0 {
                            if let Some(loot) = entry.get_loot(&params, &mut random) {
                                for stack in loot {
                                    if stack.item_count > 0 {
                                        stacks.push(stack);
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
            }
        }
        stacks
    }
}

trait LootPoolEntryExt {
    fn get_loot(
        &self,
        params: &LootContextParameters,
        random: &mut RandomGenerator,
    ) -> Option<Vec<ItemStack>>;
}

trait LootFunctionExt {
    fn apply(
        &self,
        stacks: &mut Vec<ItemStack>,
        params: &LootContextParameters,
        random: &mut RandomGenerator,
    );
}

fn apply_bonus(
    stacks: &mut [ItemStack],
    enchantment_name: &str,
    formula: &str,
    parameters: Option<&LootFunctionBonusParameter>,
    params: &LootContextParameters,
    random: &mut RandomGenerator,
) {
    let enchantment_level = params.tool.as_ref().map_or(0, |tool| {
        pumpkin_data::Enchantment::from_name(enchantment_name)
            .map_or(0, |enchantment| tool.get_enchantment_level(enchantment))
    });
    if enchantment_level > 0 {
        for stack in stacks {
            match formula {
                "minecraft:binomial_with_bonus_count" => {
                    if let Some(LootFunctionBonusParameter::Probability { extra, probability }) =
                        parameters
                    {
                        let n = enchantment_level + *extra;
                        let mut extra_items = 0;
                        for _ in 0..n {
                            if random.next_f32() < *probability {
                                extra_items += 1;
                            }
                        }
                        stack.item_count = stack.item_count.saturating_add(extra_items as u8);
                    }
                }
                "minecraft:uniform_bonus_count" => {
                    if let Some(LootFunctionBonusParameter::Multiplier { bonus_multiplier }) =
                        parameters
                    {
                        let extra =
                            random.next_bounded_i32(enchantment_level * *bonus_multiplier + 1);
                        stack.item_count = stack.item_count.saturating_add(extra as u8);
                    }
                }
                "minecraft:ore_drops" if enchantment_level > 0 => {
                    let multiplier = random.next_bounded_i32(enchantment_level + 2);
                    if multiplier > 0 {
                        stack.item_count = stack.item_count.saturating_mul(multiplier as u8);
                    }
                }
                _ => {}
            }
        }
    }
}

impl LootFunctionExt for LootFunction {
    #[allow(clippy::too_many_lines)]
    fn apply(
        &self,
        stacks: &mut Vec<ItemStack>,
        params: &LootContextParameters,
        random: &mut RandomGenerator,
    ) {
        if let Some(conditions) = self.conditions
            && !conditions
                .iter()
                .all(|cond| cond.is_fulfilled_with_rng(params, random))
        {
            return;
        }

        match &self.content {
            LootFunctionTypes::SetCount { count, add } => {
                for stack in stacks {
                    if *add {
                        stack.item_count = stack
                            .item_count
                            .saturating_add(count.generate(random).round().max(0.0) as u8);
                    } else {
                        stack.item_count = count.generate(random).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
            LootFunctionTypes::LimitCount { min, max } => {
                if let Some(min) = min.map(|min| min.round() as u8) {
                    for stack in stacks.iter_mut() {
                        if stack.item_count < min {
                            stack.item_count = min;
                        }
                    }
                }

                if let Some(max) = max.map(|max| max.round() as u8) {
                    for stack in stacks.iter_mut() {
                        if stack.item_count > max {
                            stack.item_count = max;
                        }
                    }
                }
            }
            LootFunctionTypes::ExplosionDecay => {
                if let Some(radius) = params.explosion_radius {
                    let survival_chance = 1.0 / radius;
                    for stack in stacks.iter_mut() {
                        let mut survived = 0;
                        for _ in 0..stack.item_count {
                            if random.next_f32() <= survival_chance {
                                survived += 1;
                            }
                        }
                        stack.item_count = survived;
                    }
                    // Remove empty stacks
                    stacks.retain(|stack| stack.item_count > 0);
                }
            }
            LootFunctionTypes::ApplyBonus {
                enchantment,
                formula,
                parameters,
            } => {
                apply_bonus(
                    stacks,
                    enchantment,
                    formula,
                    parameters.as_ref(),
                    params,
                    random,
                );
            }
            LootFunctionTypes::EnchantedCountIncrease {
                enchantment,
                count,
                limit,
            } => {
                let level = params.tool.as_ref().map_or(0.0, |tool| {
                    pumpkin_data::Enchantment::from_name(enchantment)
                        .map_or(0.0, |enc| tool.get_enchantment_level(enc) as f32)
                });
                let mut additional = (count.generate(random) * level).round().max(0.0) as u32;
                if let Some(lim) = limit {
                    let lim_u32 = lim.round() as u32;
                    if additional > lim_u32 {
                        additional = lim_u32;
                    }
                }
                for stack in stacks {
                    stack.item_count = stack.item_count.saturating_add(additional as u8);
                }
            }
            LootFunctionTypes::CopyComponents { source, include } => {
                tracing::warn!(
                    "CopyComponents not supported from source: {} for {:?}",
                    source,
                    include
                );
            }
            LootFunctionTypes::CopyState {
                block: _,
                properties,
            } => {
                if let Some(state) = params.block_state
                    && let Some(props_data) =
                        Block::properties(Block::from_state_id(state.id), state.id)
                {
                    let actual_props = props_data.to_props();
                    let mut properties_to_copy = std::collections::HashMap::new();
                    for &prop_name in *properties {
                        if let Some((_, value)) = actual_props.iter().find(|(k, _)| k == &prop_name)
                        {
                            properties_to_copy.insert(prop_name.to_string(), value.to_string());
                        }
                    }
                    if !properties_to_copy.is_empty() {
                        for stack in stacks.iter_mut() {
                            if let Some(block_state_comp) = stack.get_data_component_mut::<pumpkin_data::data_component_impl::BlockStateImpl>() {
                                    let mut props = block_state_comp.properties.to_mut().clone();
                                    for (k, v) in &properties_to_copy {
                                        if let Some(pos) = props.iter().position(|(pk, _)| pk.as_ref() == k) {
                                            props[pos].1 = std::borrow::Cow::Owned(v.clone());
                                        } else {
                                            props.push((std::borrow::Cow::Owned(k.clone()), std::borrow::Cow::Owned(v.clone())));
                                        }
                                    }
                                    block_state_comp.properties = std::borrow::Cow::Owned(props);
                                } else {
                                    let properties: Vec<(std::borrow::Cow<'static, str>, std::borrow::Cow<'static, str>)> = properties_to_copy
                                        .iter()
                                        .map(|(k, v)| (std::borrow::Cow::Owned(k.clone()), std::borrow::Cow::Owned(v.clone())))
                                        .collect();
                                    stack.patch.push((
                                        pumpkin_data::data_component::DataComponent::BlockState,
                                        Some(Box::new(pumpkin_data::data_component_impl::BlockStateImpl {
                                            properties: std::borrow::Cow::Owned(properties),
                                        })),
                                    ));
                                }
                        }
                    }
                }
            }
            LootFunctionTypes::SetOminousBottleAmplifier => {
                let amplifier = random.next_bounded_i32(5); // Random 0 to 4
                for stack in stacks.iter_mut() {
                    if let Some(amplifier_comp) = stack.get_data_component_mut::<pumpkin_data::data_component_impl::OminousBottleAmplifierImpl>() {
                        amplifier_comp.amplifier = amplifier;
                    } else {
                        stack.patch.push((
                            pumpkin_data::data_component::DataComponent::OminousBottleAmplifier,
                            Some(Box::new(pumpkin_data::data_component_impl::OminousBottleAmplifierImpl {
                                amplifier,
                            })),
                        ));
                    }
                }
            }
            LootFunctionTypes::SetPotion { id } => {
                let name = id.strip_prefix("minecraft:").unwrap_or(id);
                if let Some(potion) = pumpkin_data::potion::Potion::from_name(name) {
                    let potion_id = Some(potion.id as i32);
                    for stack in stacks.iter_mut() {
                        if let Some(potion_contents) = stack.get_data_component_mut::<pumpkin_data::data_component_impl::PotionContentsImpl>() {
                            potion_contents.potion_id = potion_id;
                        } else {
                            stack.patch.push((
                                pumpkin_data::data_component::DataComponent::PotionContents,
                                Some(Box::new(pumpkin_data::data_component_impl::PotionContentsImpl {
                                    potion_id,
                                    custom_color: None,
                                    custom_effects: Vec::new(),
                                    custom_name: None,
                                })),
                            ));
                        }
                    }
                }
            }
            LootFunctionTypes::FurnaceSmelt => {
                for stack in stacks.iter_mut() {
                    for recipe_type in pumpkin_data::recipes::RECIPES_COOKING {
                        if let pumpkin_data::recipes::CookingRecipeType::Smelting(recipe) =
                            recipe_type
                            && recipe.ingredient.match_item(stack.item)
                        {
                            let result_key = recipe
                                .result
                                .id
                                .strip_prefix("minecraft:")
                                .unwrap_or(recipe.result.id);
                            if let Some(smelted_item) = Item::from_registry_key(result_key) {
                                stack.item = smelted_item;
                            }
                            break;
                        }
                    }
                }
            }
        }
    }
}

impl LootPoolEntryExt for LootPoolEntry {
    fn get_loot(
        &self,
        params: &LootContextParameters,
        random: &mut RandomGenerator,
    ) -> Option<Vec<ItemStack>> {
        if let Some(conditions) = self.conditions
            && !conditions
                .iter()
                .all(|cond| cond.is_fulfilled_with_rng(params, random))
        {
            return None;
        }

        let mut stacks = self.content.get_stacks(params, random);

        if let Some(functions) = self.functions {
            for function in functions {
                function.apply(&mut stacks, params, random);
            }
        }

        Some(stacks)
    }
}

trait LootPoolEntryTypesExt {
    fn get_stacks(
        &self,
        params: &LootContextParameters,
        random: &mut RandomGenerator,
    ) -> Vec<ItemStack>;
}

impl LootPoolEntryTypesExt for LootPoolEntryTypes {
    fn get_stacks(
        &self,
        params: &LootContextParameters,
        random: &mut RandomGenerator,
    ) -> Vec<ItemStack> {
        match self {
            Self::Empty | Self::Dynamic(_) => Vec::new(),
            Self::LootTable(entry) => {
                let key = entry
                    .value
                    .strip_prefix("minecraft:")
                    .unwrap_or(entry.value);
                if key == "gameplay/fishing/fish" {
                    return generate_builtin_fish_loot(random.next_i64());
                }
                // First try chest loot tables.
                pumpkin_data::chest_loot_table::get_chest_loot_table(&format!("minecraft:{key}"))
                    .map_or_else(Vec::new, |chest_table| {
                        // We don't have a seed here, but we can generate a random one.
                        generate_chest_loot(chest_table, random.next_i64())
                    })
            }
            Self::Item(item_entry) => {
                let key = item_entry
                    .name
                    .strip_prefix("minecraft:")
                    .unwrap_or(item_entry.name);
                Item::from_registry_key(key)
                    .map_or_else(Vec::new, |item| vec![ItemStack::new(1, item)])
            }
            Self::Tag(tag) => {
                let key = tag.name.strip_prefix("minecraft:").unwrap_or(tag.name);

                let items = pumpkin_data::tag::get_tag_values(tag::RegistryKey::Item, key)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|registry_key| {
                        let item_key = registry_key
                            .strip_prefix("minecraft:")
                            .unwrap_or(registry_key);
                        Item::from_registry_key(item_key)
                    })
                    .collect::<Vec<_>>();

                if items.is_empty() {
                    return Vec::new();
                }

                if tag.expand {
                    // Pick one random item from the tag
                    let index = random.next_bounded_i32(items.len() as i32) as usize;
                    vec![ItemStack::new(1, items[index])]
                } else {
                    // Yield one stack of every item in the tag
                    items.iter().map(|&item| ItemStack::new(1, item)).collect()
                }
            }
            Self::Alternatives(alternative_entry) => {
                for entry in alternative_entry.children {
                    if let Some(loot) = entry.get_loot(params, random) {
                        return loot;
                    }
                }
                Vec::new()
            }
            Self::Sequence(sequence_entry) => {
                let mut stacks = Vec::new();
                for entry in sequence_entry.children {
                    if entry.conditions.as_ref().is_some_and(|c| {
                        !c.iter()
                            .all(|cond| cond.is_fulfilled_with_rng(params, random))
                    }) {
                        break;
                    }

                    match entry.get_loot(params, random) {
                        Some(loot) => stacks.extend(loot),
                        // get_loot returning None also signals failure — stop.
                        None => break,
                    }
                }
                stacks
            }

            Self::Group(group_entry) => {
                let mut stacks = Vec::new();
                for entry in group_entry.children {
                    if let Some(loot) = entry.get_loot(params, random) {
                        stacks.extend(loot);
                    }
                }
                stacks
            }
        }
    }
}

trait LootConditionExt {
    #[allow(dead_code)]
    fn is_fulfilled(&self, params: &LootContextParameters) -> bool;
    fn is_fulfilled_with_rng(
        &self,
        params: &LootContextParameters,
        random: &mut RandomGenerator,
    ) -> bool;
}

fn compare_entity_type(expected_type: &str, actual: &EntityType) -> bool {
    let expected = expected_type
        .strip_prefix("minecraft:")
        .unwrap_or(expected_type);
    let actual = actual
        .resource_name
        .strip_prefix("minecraft:")
        .unwrap_or(actual.resource_name);
    expected == actual
}

fn check_block_state_property(state: &BlockState, properties: &[(&str, &str)]) -> bool {
    let block_actual_properties = match Block::properties(Block::from_state_id(state.id), state.id)
    {
        Some(props_data) => props_data.to_props(), // Assuming to_props() returns HashMap<String, String>
        None => {
            return properties.is_empty();
        }
    };

    properties.iter().all(|(expected_key, expected_value)| {
        block_actual_properties
            .iter()
            .find(|(actual_key, _)| actual_key == expected_key)
            .is_some_and(|(_, actual_value_string)| actual_value_string == expected_value)
    })
}

fn check_damage_source_properties(
    params: &LootContextParameters,
    expected_source_type: Option<&str>,
    expected_direct_type: Option<&str>,
) -> bool {
    if params.damage_type.is_none() {
        return false;
    }
    if let Some(expected) = expected_source_type {
        if let Some(actual) = params.killer_entity {
            if !compare_entity_type(expected, actual) {
                return false;
            }
        } else {
            return false;
        }
    }
    if let Some(expected) = expected_direct_type {
        if let Some(actual) = params.direct_killer_entity {
            if !compare_entity_type(expected, actual) {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

impl LootConditionExt for LootCondition {
    #[allow(clippy::too_many_lines)]
    fn is_fulfilled(&self, params: &LootContextParameters) -> bool {
        let mut random = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(
            params.random_seed.unwrap_or_else(get_seed),
        ));
        self.is_fulfilled_with_rng(params, &mut random)
    }

    #[allow(clippy::too_many_lines)]
    fn is_fulfilled_with_rng(
        &self,
        params: &LootContextParameters,
        random: &mut RandomGenerator,
    ) -> bool {
        match self {
            Self::SurvivesExplosion => {
                if let Some(radius) = params.explosion_radius {
                    return random.next_f32() <= 1.0 / radius;
                }
                true
            }
            Self::RandomChance { chance } => random.next_f32() < *chance,
            Self::EntityProperties {
                entity,
                expected_type,
                is_on_fire,
                mainhand_enchantment_tag,
            } => {
                // Mirrors vanilla `EntityTarget` resolution from `LootContext.java:148-186`.
                let target = match *entity {
                    "this" => params.this_entity,
                    "attacker" | "killer" | "attacking_player" => params.killer_entity,
                    "direct_attacker" | "direct_killer" => params.direct_killer_entity,
                    _ => None,
                };
                if let Some(target) = target {
                    if let Some(expected) = expected_type
                        && !compare_entity_type(expected, target)
                    {
                        return false;
                    }
                    // Mirrors vanilla `EntityFlagsPredicate.isOnFire` check.
                    if let Some(expected_fire) = is_on_fire {
                        let actual_fire = params.is_on_fire.unwrap_or(false);
                        if actual_fire != *expected_fire {
                            return false;
                        }
                    }
                    // Mirrors vanilla enchantment tag lookup for smelts_loot.
                    if let Some(tag_name) = mainhand_enchantment_tag {
                        let tag = tag_name.strip_prefix('#').unwrap_or(tag_name);
                        let has_enchant = params.tool.as_ref().is_some_and(|tool| {
                            pumpkin_data::tag::get_tag_ids(
                                pumpkin_data::tag::RegistryKey::Enchantment,
                                tag,
                            )
                            .is_some_and(|tag_ids| {
                                tag_ids.iter().any(|&ench_id| {
                                    pumpkin_data::Enchantment::from_id(ench_id as u8)
                                        .is_some_and(|enc| tool.get_enchantment_level(enc) > 0)
                                })
                            })
                        });
                        if !has_enchant {
                            return false;
                        }
                    }
                    true
                } else {
                    false
                }
            }
            Self::KilledByPlayer => params.killed_by_player.unwrap_or(false),
            Self::BlockStateProperty {
                block: _,
                properties,
            } => {
                if let Some(state) = &params.block_state {
                    return check_block_state_property(state, properties);
                }
                false
            }
            Self::Inverted(term) => !term.is_fulfilled_with_rng(params, random),
            Self::AnyOf(terms) => terms
                .iter()
                .any(|cond| cond.is_fulfilled_with_rng(params, random)),
            Self::AllOf(terms) => terms
                .iter()
                .all(|cond| cond.is_fulfilled_with_rng(params, random)),
            Self::RandomChanceWithEnchantedBonus {
                enchantment,
                chances,
            } => chances.as_ref().is_some_and(|chances| {
                let level = params.tool.as_ref().map_or(0, |tool| {
                    pumpkin_data::Enchantment::from_name(enchantment)
                        .map_or(0, |enc| tool.get_enchantment_level(enc) as usize)
                });
                let chance = chances.get(level).unwrap_or(chances.last().unwrap_or(&0.0));
                random.next_f32() < *chance
            }),
            Self::TableBonus {
                enchantment,
                chances,
            } => {
                let level = params.tool.as_ref().map_or(0, |tool| {
                    pumpkin_data::Enchantment::from_name(enchantment)
                        .map_or(0, |enc| tool.get_enchantment_level(enc) as usize)
                });
                let chance = chances.get(level).unwrap_or(chances.last().unwrap_or(&0.0));
                random.next_f32() < *chance
            }
            Self::TimeCheck { range, period } => {
                let mut time = params.world_time;
                if let Some(period) = period {
                    time %= period;
                }
                let (min, max) = range;
                let val = time as f32;
                min.is_none_or(|min| val >= min) && max.is_none_or(|max| val <= max)
            }
            Self::ValueCheck { value, range } => {
                let val = value.get(random);
                let (min, max) = range;
                min.is_none_or(|min| val >= min) && max.is_none_or(|max| val <= max)
            }
            Self::DamageSourceProperties {
                expected_source_type,
                expected_direct_type,
            } => {
                check_damage_source_properties(params, *expected_source_type, *expected_direct_type)
            }
            Self::WeatherCheck {
                raining,
                thundering,
            } => {
                let r_match = raining.is_none_or(|r| params.is_raining.unwrap_or(false) == r);
                let t_match = thundering.is_none_or(|t| params.is_thundering.unwrap_or(false) == t);
                r_match && t_match
            }
            Self::MatchTool { items } => params.tool.as_ref().is_some_and(|tool| {
                items.as_ref().map_or_else(
                    || {
                        pumpkin_data::Enchantment::from_name("minecraft:silk_touch")
                            .is_some_and(|silk_touch| tool.get_enchantment_level(silk_touch) > 0)
                    },
                    |items| {
                        items.iter().any(|&item_name| {
                            let expected =
                                item_name.strip_prefix("minecraft:").unwrap_or(item_name);
                            let actual = tool
                                .item
                                .registry_key
                                .strip_prefix("minecraft:")
                                .unwrap_or(tool.item.registry_key);
                            expected == actual
                        })
                    },
                )
            }),
            Self::LocationCheck {
                offset_x,
                offset_y,
                offset_z,
                expected_biome,
            } => {
                let Some(expected_biome) = expected_biome else {
                    return true;
                };
                let expected = expected_biome
                    .strip_prefix("minecraft:")
                    .unwrap_or(expected_biome);
                if *offset_x != 0 || *offset_y != 0 || *offset_z != 0 {
                    let (Some(position), Some(resolve_biome)) =
                        (params.position, params.biome_resolver.as_ref())
                    else {
                        // A context without a world-backed resolver cannot
                        // answer an offset lookup.  Failing closed is safer
                        // than silently turning a datapack predicate into an
                        // unconditional match.
                        return false;
                    };
                    let position = BlockPos::new(
                        position.x.floor() as i32 + *offset_x,
                        position.y.floor() as i32 + *offset_y,
                        position.z.floor() as i32 + *offset_z,
                    );
                    return resolve_biome(position) == expected;
                }
                params.biome.is_some_and(|actual| actual == expected)
            }
            Self::EntityScores { entity } => {
                tracing::warn!("EntityScores check not supported for entity: {}", entity);
                false
            }
            Self::Reference { name } => {
                tracing::warn!("Loot condition reference not supported: {}", name);
                false
            }
            Self::EnchantmentActiveCheck { active } => {
                params.tool.as_ref().map_or(!*active, |tool| {
                    let has_enchantments = tool
                        .get_data_component::<pumpkin_data::data_component_impl::EnchantmentsImpl>()
                        .is_some_and(|e| !e.enchantment.is_empty());
                    has_enchantments == *active
                })
            }
        }
    }
}

trait LootFunctionNumberProviderExt {
    fn generate(&self, random: &mut RandomGenerator) -> f32;
}

impl LootFunctionNumberProviderExt for LootFunctionNumberProvider {
    fn generate(&self, random: &mut RandomGenerator) -> f32 {
        match self {
            Self::Constant { value } => *value,
            Self::Uniform { min, max } => random.next_f32() * (max - min) + min,
            Self::Binomial { n, p } => (0..n.floor() as u32)
                .fold(0.0, |c, _| if random.next_f32() < *p { c + 1.0 } else { c }),
        }
    }
}

/// Generates a list of items from a `ChestLootTable` using a deterministic seed.
#[must_use]
pub fn generate_chest_loot(
    table: &pumpkin_util::chest_loot_table::ChestLootTable,
    seed: i64,
) -> Vec<ItemStack> {
    use pumpkin_util::random::RandomImpl;

    let mut rng = Xoroshiro::from_seed(seed as u64);
    let mut items_to_place: Vec<ItemStack> = Vec::new();

    for pool in table.pools {
        let range = pool.max_rolls - pool.min_rolls;
        let rolls = pool.min_rolls
            + if range > 0 {
                rng.next_bounded_i32(range + 1)
            } else {
                0
            };

        for _ in 0..rolls {
            let entry_weight: i32 = pool.entries.iter().map(|e| e.weight).sum();
            let total_weight = entry_weight + pool.empty_weight;
            if total_weight == 0 {
                continue;
            }

            let mut pick = rng.next_bounded_i32(total_weight);

            // Subtract empty weight first (if the pick lands here, it yields nothing).
            pick -= pool.empty_weight;
            if pick < 0 {
                continue;
            }

            for entry in pool.entries {
                pick -= entry.weight;
                if pick < 0 {
                    let count_range = entry.max_count - entry.min_count;
                    let count = entry.min_count
                        + if count_range > 0 {
                            rng.next_bounded_i32(count_range + 1)
                        } else {
                            0
                        };

                    // Strip "minecraft:" prefix because from_registry_key uses short keys.
                    let item_key = entry.item.strip_prefix("minecraft:").unwrap_or(entry.item);

                    if let Some(item) = Item::from_registry_key(item_key) {
                        items_to_place.push(ItemStack::new(count as u8, item));
                    }
                    break;
                }
            }
        }
    }

    items_to_place
}

/// Evaluates the built-in `minecraft:gameplay/fishing` table for a hook with
/// no luck bonus.  The generated registry contains entity/block/chest tables,
/// while fishing is a separate vanilla loot-table family, so keeping this
/// small static representation here prevents fishing from silently degrading
/// to cod until the full typed datapack codec is available.  The item pools,
/// weights and open-water gate mirror `VanillaFishingLoot` and its fish/junk/
/// treasure tables; item functions that require the rod's enchantment context
/// are deliberately applied by the caller's future typed loot context.
#[must_use]
pub fn generate_builtin_fishing_loot(seed: i64, open_water: bool) -> Vec<ItemStack> {
    use pumpkin_util::random::RandomImpl;

    let mut rng = Xoroshiro::from_seed(seed as u64);
    let pool = if open_water {
        let roll = rng.next_bounded_i32(100);
        if roll < 10 {
            FishingPool::Junk
        } else if roll < 15 {
            FishingPool::Treasure
        } else {
            FishingPool::Fish
        }
    } else if rng.next_bounded_i32(95) < 10 {
        FishingPool::Junk
    } else {
        FishingPool::Fish
    };

    let item = match pool {
        FishingPool::Fish => return vec![generate_builtin_fish_stack(&mut rng)],
        FishingPool::Junk => weighted_fishing_item(
            &mut rng,
            &[
                (&Item::LILY_PAD, 17),
                (&Item::LEATHER_BOOTS, 10),
                (&Item::LEATHER, 10),
                (&Item::BONE, 10),
                (&Item::POTION, 10),
                (&Item::STRING, 5),
                (&Item::FISHING_ROD, 2),
                (&Item::BOWL, 10),
                (&Item::STICK, 5),
                (&Item::INK_SAC, 1),
                (&Item::TRIPWIRE_HOOK, 10),
                (&Item::ROTTEN_FLESH, 10),
                (&Item::BAMBOO, 10),
            ],
        ),
        FishingPool::Treasure => weighted_fishing_item(
            &mut rng,
            &[
                (&Item::NAME_TAG, 1),
                (&Item::SADDLE, 1),
                (&Item::BOW, 1),
                (&Item::FISHING_ROD, 1),
                (&Item::BOOK, 1),
                (&Item::NAUTILUS_SHELL, 1),
            ],
        ),
    };

    let mut stack = ItemStack::new(1, item);
    if item == &Item::INK_SAC {
        stack.set_count(10);
    }
    vec![stack]
}

#[derive(Clone, Copy)]
enum FishingPool {
    Fish,
    Junk,
    Treasure,
}

fn weighted_fishing_item<'a>(rng: &mut Xoroshiro, entries: &[(&'a Item, i32)]) -> &'a Item {
    let total = entries
        .iter()
        .map(|(_, weight)| *weight)
        .sum::<i32>()
        .max(1);
    let mut pick = rng.next_bounded_i32(total);
    for (item, weight) in entries {
        pick -= *weight;
        if pick < 0 {
            return item;
        }
    }
    entries.last().expect("fishing pool is non-empty").0
}

/// The nested `gameplay/fishing/fish` table is also referenced by guardian
/// entity loot. Keep that consumer on the exact fish-only pool instead of
/// applying the outer fishing junk/treasure selection.
#[must_use]
pub fn generate_builtin_fish_loot(seed: i64) -> Vec<ItemStack> {
    let mut rng = Xoroshiro::from_seed(seed as u64);
    vec![generate_builtin_fish_stack(&mut rng)]
}

fn generate_builtin_fish_stack(rng: &mut Xoroshiro) -> ItemStack {
    let item = weighted_fishing_item(
        rng,
        &[
            (&Item::COD, 60),
            (&Item::SALMON, 25),
            (&Item::TROPICAL_FISH, 2),
            (&Item::PUFFERFISH, 13),
        ],
    );
    ItemStack::new(1, item)
}

/// Evaluates a datapack-provided chest loot table. Generated vanilla tables
/// use the compact static representation above; datapacks stay owned so a
/// reload can replace them without leaking references into the old snapshot.
/// The evaluator supports the chest-table subset used by vanilla (item,
/// empty, loot_table, tag, uniform or constant rolls, set_count and
/// limit_count) and rejects unsupported shapes instead of silently deleting a
/// deferred table.
pub fn generate_datapack_chest_loot(
    resources: &crate::server::datapack::DataPackResources,
    key: &str,
    seed: i64,
) -> Result<Vec<ItemStack>, String> {
    let key = canonical_loot_key(key);
    let mut rng = Xoroshiro::from_seed(seed as u64);
    let mut stack = Vec::new();
    let mut visiting = std::collections::HashSet::new();
    roll_datapack_chest_table(resources, &key, &mut rng, &mut visiting, 0, &mut stack)?;
    Ok(stack)
}

fn canonical_loot_key(key: &str) -> String {
    if key.contains(':') {
        key.to_owned()
    } else {
        format!("minecraft:{key}")
    }
}

#[derive(Clone, Debug)]
enum DatapackChestEntryKind {
    Empty,
    Item(String),
    LootTable(String),
    Tag(String, bool),
}

#[derive(Clone, Debug)]
struct DatapackChestEntry {
    kind: DatapackChestEntryKind,
    weight: i32,
    min_count: i32,
    max_count: i32,
}

#[derive(Clone, Debug)]
struct DatapackChestPool {
    entries: Vec<DatapackChestEntry>,
    min_rolls: i32,
    max_rolls: i32,
}

fn roll_datapack_chest_table(
    resources: &crate::server::datapack::DataPackResources,
    key: &str,
    rng: &mut Xoroshiro,
    visiting: &mut std::collections::HashSet<String>,
    depth: usize,
    output: &mut Vec<ItemStack>,
) -> Result<(), String> {
    const MAX_NESTED_TABLE_DEPTH: usize = 32;
    if depth >= MAX_NESTED_TABLE_DEPTH {
        return Err(format!(
            "loot table nesting exceeds {MAX_NESTED_TABLE_DEPTH}"
        ));
    }
    if !visiting.insert(key.to_owned()) {
        return Err(format!("recursive loot table reference: {key}"));
    }
    let result = (|| {
        let value = resources
            .loot_tables
            .get(key)
            .ok_or_else(|| format!("loot table {key} is not loaded"))?;
        let pools = value
            .get("pools")
            .and_then(Value::as_array)
            .ok_or_else(|| "loot table pools must be an array".to_owned())?;
        for pool in pools {
            let parsed = parse_datapack_chest_pool(pool)?;
            // Deferred chest and /loot contexts have no luck source. Vanilla
            // multiplies bonus_rolls by luck and floors the result, therefore
            // bonus rolls are zero here even when the provider is present.
            let rolls = sample_range(parsed.min_rolls, parsed.max_rolls, rng);
            for _ in 0..rolls.max(0) {
                let total_weight = parsed
                    .entries
                    .iter()
                    .map(|entry| entry.weight.max(0))
                    .fold(0i32, i32::saturating_add);
                if total_weight <= 0 {
                    continue;
                }
                let mut choice = rng.next_bounded_i32(total_weight);
                let Some(entry) = parsed.entries.iter().find(|entry| {
                    choice -= entry.weight.max(0);
                    choice < 0
                }) else {
                    continue;
                };
                roll_datapack_chest_entry(resources, entry, rng, visiting, depth, output)?;
            }
        }
        Ok::<(), String>(())
    })();
    visiting.remove(key);
    result
}

fn roll_datapack_chest_entry(
    resources: &crate::server::datapack::DataPackResources,
    entry: &DatapackChestEntry,
    rng: &mut Xoroshiro,
    visiting: &mut std::collections::HashSet<String>,
    depth: usize,
    output: &mut Vec<ItemStack>,
) -> Result<(), String> {
    match &entry.kind {
        DatapackChestEntryKind::Empty => Ok(()),
        DatapackChestEntryKind::LootTable(key) => roll_datapack_chest_table(
            resources,
            &canonical_loot_key(key),
            rng,
            visiting,
            depth + 1,
            output,
        ),
        DatapackChestEntryKind::Item(item_key) => {
            let key = item_key.strip_prefix("minecraft:").unwrap_or(item_key);
            let Some(item) = Item::from_registry_key(key) else {
                return Err(format!("loot table references unknown item {item_key}"));
            };
            let count =
                sample_range(entry.min_count, entry.max_count, rng).clamp(1, u8::MAX as i32);
            output.push(ItemStack::new(count as u8, item));
            Ok(())
        }
        DatapackChestEntryKind::Tag(tag_key, expand) => {
            let key = tag_key.strip_prefix("minecraft:").unwrap_or(tag_key);
            let values =
                pumpkin_data::tag::get_tag_values(tag::RegistryKey::Item, key).unwrap_or_default();
            let items = values
                .iter()
                .filter_map(|value| {
                    Item::from_registry_key(value.strip_prefix("minecraft:").unwrap_or(value))
                })
                .collect::<Vec<_>>();
            if *expand && !items.is_empty() {
                if let Some(item) = items
                    .get(rng.next_bounded_i32(items.len().try_into().unwrap_or(i32::MAX)) as usize)
                {
                    let count = sample_range(entry.min_count, entry.max_count, rng)
                        .clamp(1, u8::MAX as i32);
                    output.push(ItemStack::new(count as u8, item));
                }
            } else {
                for item in items {
                    let count = sample_range(entry.min_count, entry.max_count, rng)
                        .clamp(1, u8::MAX as i32);
                    output.push(ItemStack::new(count as u8, item));
                }
            }
            Ok(())
        }
    }
}

fn parse_datapack_chest_pool(value: &Value) -> Result<DatapackChestPool, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "loot pool must be an object".to_owned())?;
    let entries = object
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "loot pool entries must be an array".to_owned())?
        .iter()
        .map(parse_datapack_chest_entry)
        .collect::<Result<Vec<_>, _>>()?;
    if object
        .get("conditions")
        .is_some_and(|conditions| !conditions.as_array().is_some_and(Vec::is_empty))
    {
        return Err("loot pool conditions are not supported by the deferred subset".to_owned());
    }
    let (min_rolls, max_rolls) = parse_integer_range(object.get("rolls"), (1, 1), "rolls")?;
    // Validate the optional provider even though a chest has luck=0 and thus
    // cannot produce bonus rolls. This prevents malformed data from surfacing
    // only after a player opens the chest.
    let _ = parse_integer_range(object.get("bonus_rolls"), (0, 0), "bonus_rolls")?;
    Ok(DatapackChestPool {
        entries,
        min_rolls,
        max_rolls,
    })
}

fn parse_datapack_chest_entry(value: &Value) -> Result<DatapackChestEntry, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "loot entry must be an object".to_owned())?;
    let entry_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "loot entry type must be a string".to_owned())?;
    if object
        .get("conditions")
        .is_some_and(|conditions| !conditions.as_array().is_some_and(Vec::is_empty))
    {
        return Err("loot entry conditions are not supported by the deferred subset".to_owned());
    }
    let kind = match entry_type {
        "minecraft:empty" | "empty" => DatapackChestEntryKind::Empty,
        "minecraft:item" | "item" => DatapackChestEntryKind::Item(
            object
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "item loot entry is missing name".to_owned())?
                .to_owned(),
        ),
        "minecraft:loot_table" | "loot_table" => DatapackChestEntryKind::LootTable(
            object
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| "loot_table entry is missing value".to_owned())?
                .to_owned(),
        ),
        "minecraft:tag" | "tag" => DatapackChestEntryKind::Tag(
            object
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "tag loot entry is missing name".to_owned())?
                .to_owned(),
            object
                .get("expand")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
        other => return Err(format!("unsupported chest loot entry type {other}")),
    };
    let weight = object
        .get("weight")
        .and_then(Value::as_i64)
        .unwrap_or(1)
        .clamp(0, i64::from(i32::MAX)) as i32;
    let mut count: (i32, i32) = (1, 1);
    if let Some(functions) = object.get("functions").and_then(Value::as_array) {
        for function in functions {
            let function_object = function
                .as_object()
                .ok_or_else(|| "loot function must be an object".to_owned())?;
            match function_object.get("function").and_then(Value::as_str) {
                Some("minecraft:set_count" | "set_count") => {
                    let parsed =
                        parse_integer_range(function_object.get("count"), (1, 1), "count")?;
                    let add = function_object
                        .get("add")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    count = if add {
                        (
                            count.0.saturating_add(parsed.0),
                            count.1.saturating_add(parsed.1),
                        )
                    } else {
                        parsed
                    };
                }
                Some("minecraft:limit_count" | "limit_count") => {
                    let limits = function_object
                        .get("limit")
                        .and_then(Value::as_object)
                        .ok_or_else(|| "limit_count function is missing limit".to_owned())?;
                    if let Some(min) = limits.get("min").and_then(Value::as_i64) {
                        count.0 = count
                            .0
                            .max(min.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32);
                        count.1 = count.1.max(count.0);
                    }
                    if let Some(max) = limits.get("max").and_then(Value::as_i64) {
                        let max = max.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                        count.0 = count.0.min(max);
                        count.1 = count.1.min(max).max(count.0);
                    }
                }
                Some(other) => return Err(format!("unsupported chest loot function {other}")),
                None => return Err("loot function is missing function".to_owned()),
            }
        }
    }
    Ok(DatapackChestEntry {
        kind,
        weight,
        min_count: count.0,
        max_count: count.1,
    })
}

fn parse_integer_range(
    value: Option<&Value>,
    default: (i32, i32),
    field: &str,
) -> Result<(i32, i32), String> {
    let Some(value) = value else {
        return Ok(default);
    };
    let (min, max) = if let Some(number) = value.as_f64() {
        (number, number)
    } else if let Some(object) = value.as_object() {
        let min = object
            .get("min")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("{field} provider is missing min"))?;
        let max = object
            .get("max")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("{field} provider is missing max"))?;
        (min, max)
    } else {
        return Err(format!("{field} must be a number or uniform provider"));
    };
    if !min.is_finite() || !max.is_finite() || min > max {
        return Err(format!("{field} range is invalid"));
    }
    let min = min.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let max = max.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    Ok((min, max.max(min)))
}

fn sample_range(min: i32, max: i32, rng: &mut Xoroshiro) -> i32 {
    let max = max.max(min);
    let range = max.saturating_sub(min);
    min.saturating_add(if range == 0 {
        0
    } else {
        rng.next_bounded_i32(range.saturating_add(1))
    })
}

/// Items are scattered randomly across the 27 chest slots.
pub async fn fill_chest_inventory(
    inventory: &std::sync::Arc<dyn pumpkin_world::inventory::Inventory>,
    table: &pumpkin_util::chest_loot_table::ChestLootTable,
    seed: i64,
) {
    let items_to_place = generate_chest_loot(table, seed);
    fill_chest_inventory_items(inventory, items_to_place, seed).await;
}

/// Places already-evaluated loot into a chest using vanilla's deterministic
/// split/shuffle pass. Keeping generation and placement separate lets dynamic
/// datapack tables use exactly the same inventory semantics as built-ins.
pub async fn fill_chest_inventory_items(
    inventory: &std::sync::Arc<dyn pumpkin_world::inventory::Inventory>,
    mut items_to_place: Vec<ItemStack>,
    seed: i64,
) {
    if items_to_place.is_empty() {
        return;
    }

    let inv_size = inventory.size(); // 27 for a normal chest
    let mut rng = Xoroshiro::from_seed(seed as u64);
    let free_slots = inv_size;

    // Split large stacks across extra slots then shuffle.
    shuffle_and_split_items(&mut items_to_place, free_slots, &mut rng);

    // Pick random distinct slots and place each item.
    let mut available_slots: Vec<usize> = (0..inv_size).collect();
    // Shuffle available slots using Fisher-Yates so item order from above maps to random slots.
    for i in (1..available_slots.len()).rev() {
        let j = rng.next_bounded_i32((i + 1) as i32) as usize;
        available_slots.swap(i, j);
    }

    for item in items_to_place {
        let Some(slot) = available_slots.pop() else {
            break;
        };
        inventory.set_stack(slot, item).await;
    }
}

/// Stacks with count > 1 are split at a random midpoint and redistributed while
/// there are more free slots than total items. Then everything is shuffled.
fn shuffle_and_split_items(
    result: &mut Vec<ItemStack>,
    available_slots: usize,
    rng: &mut Xoroshiro,
) {
    use pumpkin_util::random::RandomImpl;

    // Drain all items with count > 1 into a splittable list.
    let mut splittable: Vec<ItemStack> = Vec::new();
    let mut i = 0;
    while i < result.len() {
        if result[i].item_count > 1 {
            splittable.push(result.swap_remove(i));
        } else {
            i += 1;
        }
    }

    // While there are more free slots than total items, split a random stack.
    while available_slots > result.len() + splittable.len() && !splittable.is_empty() {
        let idx = rng.next_bounded_i32(splittable.len() as i32) as usize;
        let mut stack = splittable.swap_remove(idx);

        let count = stack.item_count as i32;
        // Split off [1, count/2] items.
        let split_off = 1 + rng.next_bounded_i32(count / 2);
        stack.item_count = (count - split_off) as u8;
        let mut copy = stack.clone();
        copy.item_count = split_off as u8;

        if stack.item_count > 1 {
            splittable.push(stack);
        } else {
            result.push(stack);
        }
        if copy.item_count > 1 {
            splittable.push(copy);
        } else {
            result.push(copy);
        }
    }

    // Remaining unsplit multis go straight into result.
    result.extend(splittable);

    // Fisher-Yates shuffle with our RNG.
    let n = result.len();
    for i in (1..n).rev() {
        let j = rng.next_bounded_i32((i + 1) as i32) as usize;
        result.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::Enchantment;
    use pumpkin_data::damage::DamageType;
    use pumpkin_data::entity::EntityType;
    use pumpkin_data::item::Item;
    use pumpkin_data::item_stack::ItemStack;
    use pumpkin_util::loot_table::{
        ItemEntry, LootFunctionTypes, LootNumberProviderTypes, LootPool, LootPoolEntry,
        LootPoolEntryTypes, LootTableEntry, LootTableType,
    };
    use serde_json::json;

    fn base_params() -> LootContextParameters {
        LootContextParameters {
            killed_by_player: Some(true),
            this_entity: Some(&EntityType::PIG),
            killer_entity: Some(&EntityType::PLAYER),
            direct_killer_entity: Some(&EntityType::PLAYER),
            damage_type: Some(DamageType::GENERIC),
            ..Default::default()
        }
    }

    #[test]
    fn datapack_chest_loot_is_seeded_and_applies_count_functions() {
        let mut resources = crate::server::datapack::DataPackResources::default();
        resources.loot_tables.insert(
            "example:test".to_owned(),
            json!({
                "type": "minecraft:chest",
                "pools": [{
                    "rolls": 1,
                    "entries": [{
                        "type": "minecraft:item",
                        "name": "minecraft:diamond",
                        "functions": [{
                            "function": "minecraft:set_count",
                            "count": {"min": 2, "max": 2}
                        }]
                    }]
                }]
            }),
        );

        let first = generate_datapack_chest_loot(&resources, "example:test", 42)
            .expect("valid datapack table");
        let second = generate_datapack_chest_loot(&resources, "example:test", 42)
            .expect("valid datapack table");
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), first.len());
        assert_eq!(first[0].item.id, second[0].item.id);
        assert_eq!(first[0].item_count, second[0].item_count);
        assert_eq!(first[0].item.registry_key, "diamond");
        assert_eq!(first[0].item_count, 2);
    }

    #[test]
    fn datapack_chest_loot_rejects_recursive_tables_without_partial_output() {
        let mut resources = crate::server::datapack::DataPackResources::default();
        resources.loot_tables.insert(
            "example:a".to_owned(),
            json!({
                "pools": [{
                    "rolls": 1,
                    "entries": [{
                        "type": "minecraft:loot_table",
                        "value": "example:b"
                    }]
                }]
            }),
        );
        resources.loot_tables.insert(
            "example:b".to_owned(),
            json!({
                "pools": [{
                    "rolls": 1,
                    "entries": [{
                        "type": "minecraft:loot_table",
                        "value": "example:a"
                    }]
                }]
            }),
        );

        let error = match generate_datapack_chest_loot(&resources, "example:a", 42) {
            Ok(_) => panic!("recursive tables must be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("recursive loot table reference"));
    }

    #[test]
    fn builtin_fishing_loot_is_seeded_and_not_cod_only() {
        let first = generate_builtin_fishing_loot(42, false);
        let second = generate_builtin_fishing_loot(42, false);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].item.id, second[0].item.id);
        assert_eq!(first[0].item_count, second[0].item_count);
        assert!(
            [
                Item::COD.id,
                Item::SALMON.id,
                Item::TROPICAL_FISH.id,
                Item::PUFFERFISH.id,
                Item::LILY_PAD.id,
                Item::LEATHER_BOOTS.id,
                Item::LEATHER.id,
                Item::BONE.id,
                Item::POTION.id,
                Item::STRING.id,
                Item::FISHING_ROD.id,
                Item::BOWL.id,
                Item::STICK.id,
                Item::INK_SAC.id,
                Item::TRIPWIRE_HOOK.id,
                Item::ROTTEN_FLESH.id,
                Item::BAMBOO.id,
            ]
            .contains(&first[0].item.id)
        );
    }

    #[test]
    fn deferred_subset_rejects_conditions_instead_of_ignoring_them() {
        let mut resources = crate::server::datapack::DataPackResources::default();
        resources.loot_tables.insert(
            "example:conditional".to_owned(),
            json!({
                "pools": [{
                    "conditions": [{"condition": "minecraft:random_chance", "chance": 0.0}],
                    "entries": [{"type": "minecraft:item", "name": "minecraft:diamond"}]
                }]
            }),
        );
        let error = match generate_datapack_chest_loot(&resources, "example:conditional", 1) {
            Ok(_) => panic!("unsupported conditions must fail closed"),
            Err(error) => error,
        };
        assert!(error.contains("conditions are not supported"));
    }

    #[test]
    fn nested_guardian_fish_table_returns_fish_only() {
        static ENTRIES: [LootPoolEntry; 1] = [LootPoolEntry {
            content: LootPoolEntryTypes::LootTable(LootTableEntry {
                value: "minecraft:gameplay/fishing/fish",
            }),
            weight: 1,
            quality: 0,
            conditions: None,
            functions: None,
        }];
        static POOLS: [LootPool; 1] = [LootPool {
            entries: &ENTRIES,
            rolls: LootNumberProviderTypes::Constant(1.0),
            bonus_rolls: LootNumberProviderTypes::Constant(0.0),
            conditions: None,
            functions: None,
        }];
        static TABLE: LootTable = LootTable {
            r#type: LootTableType::Entity,
            random_sequence: None,
            pools: Some(&POOLS),
        };

        let loot = TABLE.get_loot(LootContextParameters {
            random_seed: Some(123),
            ..base_params()
        });
        assert_eq!(loot.len(), 1);
        assert!(
            [
                Item::COD.id,
                Item::SALMON.id,
                Item::TROPICAL_FISH.id,
                Item::PUFFERFISH.id,
            ]
            .contains(&loot[0].item.id)
        );
    }

    fn fire_aspect_sword(level: i32) -> ItemStack {
        let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        sword.enchant(&Enchantment::FIRE_ASPECT, level);
        sword
    }

    #[test]
    fn seeded_loot_uses_one_repeatable_random_stream() {
        static FUNCTIONS: [LootFunction; 1] = [LootFunction {
            content: LootFunctionTypes::SetCount {
                count: LootFunctionNumberProvider::Uniform {
                    min: 1.0,
                    max: 16.0,
                },
                add: false,
            },
            conditions: None,
        }];
        static ENTRIES: [LootPoolEntry; 1] = [LootPoolEntry {
            content: LootPoolEntryTypes::Item(ItemEntry {
                name: "minecraft:stone",
            }),
            weight: 1,
            quality: 0,
            conditions: None,
            functions: Some(&FUNCTIONS),
        }];
        static POOLS: [LootPool; 1] = [LootPool {
            entries: &ENTRIES,
            rolls: LootNumberProviderTypes::Constant(4.0),
            bonus_rolls: LootNumberProviderTypes::Constant(0.0),
            conditions: None,
            functions: None,
        }];
        static TABLE: LootTable = LootTable {
            r#type: LootTableType::Chest,
            random_sequence: Some("test:seeded"),
            pools: Some(&POOLS),
        };

        let first = TABLE.get_loot(LootContextParameters {
            random_seed: Some(0x5eed),
            ..Default::default()
        });
        let second = TABLE.get_loot(LootContextParameters {
            random_seed: Some(0x5eed),
            ..Default::default()
        });
        let different = TABLE.get_loot(LootContextParameters {
            random_seed: Some(0x5eee),
            ..Default::default()
        });

        let signature = |stacks: &[ItemStack]| {
            stacks
                .iter()
                .map(|stack| (stack.item.id, stack.item_count))
                .collect::<Vec<_>>()
        };
        assert_eq!(signature(&first), signature(&second));
        assert_ne!(signature(&first), signature(&different));
    }

    #[test]
    fn derived_loot_seed_is_stable_and_source_separated() {
        let position = pumpkin_util::math::vector3::Vector3::new(3.0, 64.0, -9.0);
        let first = derive_loot_seed(1234, Some(position), 77, 1);
        assert_eq!(first, derive_loot_seed(1234, Some(position), 77, 1));
        assert_ne!(first, derive_loot_seed(1234, Some(position), 77, 2));
        assert_ne!(first, derive_loot_seed(1234, Some(position), 78, 1));
        assert_ne!(first, derive_loot_seed(1235, Some(position), 77, 1));
    }

    #[test]
    fn location_check_uses_context_biome_and_fails_closed_for_offsets() {
        let params = LootContextParameters {
            biome: Some("plains"),
            ..Default::default()
        };
        let matches = LootCondition::LocationCheck {
            offset_x: 0,
            offset_y: 0,
            offset_z: 0,
            expected_biome: Some("minecraft:plains"),
        };
        let wrong = LootCondition::LocationCheck {
            offset_x: 0,
            offset_y: 0,
            offset_z: 0,
            expected_biome: Some("minecraft:desert"),
        };
        let offset = LootCondition::LocationCheck {
            offset_x: 1,
            offset_y: 0,
            offset_z: 0,
            expected_biome: Some("minecraft:plains"),
        };
        assert!(matches.is_fulfilled(&params));
        assert!(!wrong.is_fulfilled(&params));
        assert!(!offset.is_fulfilled(&params));
    }

    #[test]
    fn location_check_resolves_non_zero_offset_from_containing_block() {
        fn resolve(position: BlockPos) -> &'static str {
            if position == BlockPos::new(11, 64, 2) {
                "plains"
            } else {
                "desert"
            }
        }

        let params = LootContextParameters {
            position: Some(pumpkin_util::math::vector3::Vector3::new(10.9, 64.9, 2.1)),
            biome_resolver: Some(Arc::new(resolve)),
            ..Default::default()
        };
        let offset = LootCondition::LocationCheck {
            offset_x: 1,
            offset_y: 0,
            offset_z: 0,
            expected_biome: Some("minecraft:plains"),
        };
        assert!(offset.is_fulfilled(&params));
    }

    #[test]
    fn entity_properties_this_matches_expected_type() {
        let params = base_params();
        let cond = LootCondition::EntityProperties {
            entity: "this",
            expected_type: Some("minecraft:pig"),
            is_on_fire: None,
            mainhand_enchantment_tag: None,
        };
        assert!(cond.is_fulfilled(&params));
    }

    #[test]
    fn entity_properties_this_rejects_wrong_type() {
        let params = base_params();
        let cond = LootCondition::EntityProperties {
            entity: "this",
            expected_type: Some("minecraft:cow"),
            is_on_fire: None,
            mainhand_enchantment_tag: None,
        };
        assert!(!cond.is_fulfilled(&params));
    }

    #[test]
    fn entity_properties_direct_attacker_resolves() {
        let params = base_params();
        let cond = LootCondition::EntityProperties {
            entity: "direct_attacker",
            expected_type: None,
            is_on_fire: None,
            mainhand_enchantment_tag: None,
        };
        assert!(cond.is_fulfilled(&params));
    }

    #[test]
    fn entity_properties_direct_attacker_no_direct_killer() {
        let mut params = base_params();
        params.direct_killer_entity = None;
        let cond = LootCondition::EntityProperties {
            entity: "direct_attacker",
            expected_type: None,
            is_on_fire: None,
            mainhand_enchantment_tag: None,
        };
        assert!(!cond.is_fulfilled(&params));
    }

    #[test]
    fn entity_properties_unknown_entity_returns_false() {
        let params = base_params();
        let cond = LootCondition::EntityProperties {
            entity: "target_entity",
            expected_type: None,
            is_on_fire: None,
            mainhand_enchantment_tag: None,
        };
        assert!(!cond.is_fulfilled(&params));
    }

    #[test]
    fn is_on_fire_true_when_burning() {
        let params = LootContextParameters {
            is_on_fire: Some(true),
            ..base_params()
        };
        let cond = LootCondition::EntityProperties {
            entity: "this",
            expected_type: None,
            is_on_fire: Some(true),
            mainhand_enchantment_tag: None,
        };
        assert!(cond.is_fulfilled(&params));
    }

    #[test]
    fn is_on_fire_true_fails_when_not_burning() {
        let params = LootContextParameters {
            is_on_fire: Some(false),
            ..base_params()
        };
        let cond = LootCondition::EntityProperties {
            entity: "this",
            expected_type: None,
            is_on_fire: Some(true),
            mainhand_enchantment_tag: None,
        };
        assert!(!cond.is_fulfilled(&params));
    }

    #[test]
    fn is_on_fire_false_matches_not_burning() {
        let params = LootContextParameters {
            is_on_fire: Some(false),
            ..base_params()
        };
        let cond = LootCondition::EntityProperties {
            entity: "this",
            expected_type: None,
            is_on_fire: Some(false),
            mainhand_enchantment_tag: None,
        };
        assert!(cond.is_fulfilled(&params));
    }

    #[test]
    fn is_on_fire_true_fails_when_context_none() {
        let params = LootContextParameters {
            is_on_fire: None,
            ..base_params()
        };
        let cond = LootCondition::EntityProperties {
            entity: "this",
            expected_type: None,
            is_on_fire: Some(true),
            mainhand_enchantment_tag: None,
        };
        assert!(!cond.is_fulfilled(&params));
    }

    #[test]
    fn none_is_on_fire_skips_check() {
        let params = LootContextParameters {
            is_on_fire: Some(true),
            ..base_params()
        };
        let cond = LootCondition::EntityProperties {
            entity: "this",
            expected_type: None,
            is_on_fire: None,
            mainhand_enchantment_tag: None,
        };
        assert!(cond.is_fulfilled(&params));
    }

    #[test]
    fn enchantment_tag_matches_fire_aspect() {
        let params = LootContextParameters {
            tool: Some(fire_aspect_sword(1)),
            ..base_params()
        };
        let cond = LootCondition::EntityProperties {
            entity: "direct_attacker",
            expected_type: None,
            is_on_fire: None,
            mainhand_enchantment_tag: Some("minecraft:smelts_loot"),
        };
        assert!(cond.is_fulfilled(&params));
    }

    #[test]
    fn enchantment_tag_fails_without_enchantment() {
        let params = LootContextParameters {
            tool: Some(ItemStack::new(1, &Item::DIAMOND_SWORD)),
            ..base_params()
        };
        let cond = LootCondition::EntityProperties {
            entity: "direct_attacker",
            expected_type: None,
            is_on_fire: None,
            mainhand_enchantment_tag: Some("minecraft:smelts_loot"),
        };
        assert!(!cond.is_fulfilled(&params));
    }

    #[test]
    fn enchantment_tag_rejects_unrelated_enchantment() {
        let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        sword.enchant(&Enchantment::SHARPNESS, 5);
        let params = LootContextParameters {
            tool: Some(sword),
            ..base_params()
        };
        let cond = LootCondition::EntityProperties {
            entity: "direct_attacker",
            expected_type: None,
            is_on_fire: None,
            mainhand_enchantment_tag: Some("minecraft:smelts_loot"),
        };
        assert!(!cond.is_fulfilled(&params));
    }

    #[test]
    fn enchantment_tag_fails_with_no_tool() {
        let params = LootContextParameters {
            tool: None,
            ..base_params()
        };
        let cond = LootCondition::EntityProperties {
            entity: "direct_attacker",
            expected_type: None,
            is_on_fire: None,
            mainhand_enchantment_tag: Some("minecraft:smelts_loot"),
        };
        assert!(!cond.is_fulfilled(&params));
    }

    #[test]
    fn none_enchantment_tag_skips_check() {
        let params = LootContextParameters {
            tool: Some(fire_aspect_sword(2)),
            ..base_params()
        };
        let cond = LootCondition::EntityProperties {
            entity: "direct_attacker",
            expected_type: None,
            is_on_fire: None,
            mainhand_enchantment_tag: None,
        };
        assert!(cond.is_fulfilled(&params));
    }

    #[test]
    fn anyof_passes_when_entity_on_fire() {
        let params = LootContextParameters {
            is_on_fire: Some(true),
            tool: Some(ItemStack::new(1, &Item::DIAMOND_SWORD)),
            ..base_params()
        };
        let cond = LootCondition::AnyOf(&[
            LootCondition::EntityProperties {
                entity: "this",
                expected_type: None,
                is_on_fire: Some(true),
                mainhand_enchantment_tag: None,
            },
            LootCondition::EntityProperties {
                entity: "direct_attacker",
                expected_type: None,
                is_on_fire: None,
                mainhand_enchantment_tag: Some("minecraft:smelts_loot"),
            },
        ]);
        assert!(cond.is_fulfilled(&params));
    }

    #[test]
    fn anyof_passes_when_weapon_has_fire_aspect() {
        let params = LootContextParameters {
            is_on_fire: Some(false),
            tool: Some(fire_aspect_sword(1)),
            ..base_params()
        };
        let cond = LootCondition::AnyOf(&[
            LootCondition::EntityProperties {
                entity: "this",
                expected_type: None,
                is_on_fire: Some(true),
                mainhand_enchantment_tag: None,
            },
            LootCondition::EntityProperties {
                entity: "direct_attacker",
                expected_type: None,
                is_on_fire: None,
                mainhand_enchantment_tag: Some("minecraft:smelts_loot"),
            },
        ]);
        assert!(cond.is_fulfilled(&params));
    }

    #[test]
    fn anyof_fails_without_fire_or_fire_aspect() {
        let params = LootContextParameters {
            is_on_fire: Some(false),
            tool: Some(ItemStack::new(1, &Item::DIAMOND_SWORD)),
            ..base_params()
        };
        let cond = LootCondition::AnyOf(&[
            LootCondition::EntityProperties {
                entity: "this",
                expected_type: None,
                is_on_fire: Some(true),
                mainhand_enchantment_tag: None,
            },
            LootCondition::EntityProperties {
                entity: "direct_attacker",
                expected_type: None,
                is_on_fire: None,
                mainhand_enchantment_tag: Some("minecraft:smelts_loot"),
            },
        ]);
        assert!(!cond.is_fulfilled(&params));
    }
}
