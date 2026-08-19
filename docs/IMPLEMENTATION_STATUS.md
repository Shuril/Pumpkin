# Implementation status

This checkout is buildable and the implemented contracts are covered by the
tests listed below. `FULL_IMPLEMENTATION_PLAN.md` remains the acceptance
specification; a row is not considered complete merely because its type or
registry entry exists.

## Implemented in this batch

- `registry.identifier`: resource identifiers now reject empty namespaces and
  paths (including explicit `:path`/`namespace:` forms), apply the `minecraft`
  namespace only to unqualified input, and keep canonical namespaced display
  through serde/codec parsing.
- Bedrock actor metadata now serializes CompoundTag values using the Bedrock
  network-NBT writer; the previous branch returned an error and silently made
  entity metadata updates incomplete.
- `recipe.crafter`: vanilla and dynamic shaped/shapeless matching is shared
  with normal crafting, disabled slots are persisted, one item per enabled
  input slot is consumed, output is inserted into the adjacent inventory when
  possible and otherwise dropped in the crafter's orientation direction;
  destination insertion now honors the output face's sided-container rules and
  merge vetoes.
  Dynamic datapack results preserve component patches through matching,
  recipe-book display and output construction; incompatible recipe remainders
  are returned to the player inventory or dropped instead of being deleted.
  Crafter input consumption now validates a complete slot snapshot while
  holding one write transaction, so a concurrent menu/hopper mutation aborts
  the pulse without consuming a different recipe.  Crafter remainders prefer
  the item's `use_remainder` component, preserve its declared count, and fall
  back to the generated legacy table for older item definitions; the result
  and every remainder are emitted in vanilla order through the crafter's
  front-container/drop path, and hopper insertion rejects disabled slots while
  preserving vanilla lower-count slot ordering.
  Decorated-pot outputs now carry the four-sherd `pot_decorations` component;
  transmute/map-cloning recipes honor material-count bounds and copy the input
  component patch into the result.
- Crafter success now leaves the `CRAFTING` block state visible for the full
  six-tick block-entity countdown; the completion tick clears it, marks the
  entity dirty and refreshes comparator neighbours. Failed pulses never enter
  the animation state.
- Dispenser brush behavior now handles an armadillo in the front one-block
  target box first, drops one scute, applies the vanilla 16 durability cost,
  emits the interaction event, and only then falls back to suspicious
  archaeology/default eject behavior.
- `recipe.server_book`: dynamic recipe IDs are canonicalized and duplicate
  registrations replace the previous definition; player recipe keys,
  `toBeDisplayed` highlights and the four category open/filter settings are
  persisted in vanilla `recipeBook` NBT; serverbound category settings are
  applied and reconnect sends the saved settings. Dynamic crafting recipes
  now retain their own namespaced key instead of using the output item as an
  identity. Limited-crafting checks share one canonical helper between
  result-slot commits and Java `PlaceRecipe` packets; stale window IDs,
  malformed display IDs and unknown recipe keys are rejected before inventory
  mutation. The active generated+datapack registry is now the source of truth
  at player-load and `/reload` boundaries: unknown persisted keys are ignored,
  removed recipes are pruned atomically from known/highlight sets, and Java
  clients receive a rebuilt recipe-display registry. Recipe-book packets now
  filter generated and dynamic displays by the player's discovered keys while
  preserving global display IDs; `/recipe give` and `/recipe take` cover both
  generated vanilla and datapack recipes.
- Java `PlaceRecipe` now handles furnace, blast-furnace and smoker displays for
  both generated and datapack cooking recipes, using the one-slot input
  inventory and rejecting recipes for the wrong menu type. Campfire-only
  recipes remain unavailable because Pumpkin has no campfire recipe screen.
- `persistence.player_inventory`: new saves use Mojang's `Inventory` slot
  numbers (`0..35`, off-hand `40`, armor `100..103`); old Pumpkin equipment
  compounds remain readable.
- `persistence.player_unknown_nbt`: player autosaves merge an opaque snapshot
  of root tags unknown to this runtime before writing live fields, so newer
  vanilla/datapack state is not discarded on reconnect or cron save.
- `persistence.data_file_extensions`: weather, game-rule, world-generation,
  clock and wandering-trader data-file rewrites start from the existing NBT
  document and update recognized values without deleting unknown extensions.
  Custom boss-event and global scheduled-event files now follow the same
  lossless upgrade path (including DataVersion and a missing empty event list).
  The live custom-bossbar registry now loads on server startup and writes on
  shutdown; malformed existing files are never overwritten after a failed
  load.
- `world.game_event`: a shared event/frequency model, bounded vibration
  dispatch, calibrated horizontal-direction filtering and Warden summon path
  are present. Block placement and destruction call the dispatcher at their
  authoritative boundaries; shriekers now use the vanilla 20-attempt
  `ON_TOP_OF_COLLIDER` vertical search instead of a single random Y sample.
  Live mounts and entity dismounts now emit `ENTITY_MOUNT`/`ENTITY_DISMOUNT`
  from the vehicle position before client movement; persisted passenger-tree
  hydration uses a silent path so chunk loading does not replay gameplay
  vibrations. Living damage and one-shot death now emit `ENTITY_DAMAGE` and
  `ENTITY_DIE` only after the corresponding state transition is authoritative.
  Vehicles (including TNT minecarts on their specialised prime path) and item
  entities now emit `ENTITY_DAMAGE`; item entities also honor the vanilla
  `mob_griefing` guard for mob-caused damage.
- `entity.creeper_lingering_cloud`: creeper explosions snapshot active effects
  before removing the mob and create a vanilla-parameterized area-effect cloud
  only when the snapshot is non-empty. Effect duration, amplifier and visual
  flags are copied into the cloud; pure conversion tests cover both populated
  and empty snapshots.
- `entity.fire_clears_freeze`: igniting an entity now preserves the longer
  existing fire duration, clears frozen ticks like vanilla `clearFreeze`, and
  saturates oversized durations instead of wrapping them into a negative
  timer. The state transition has pure regression coverage.
- `mob.blaze_attack_visibility`: Blaze goals now reject dead targets, use the
  collision-shape raycast for line-of-sight, perform close-range attacks via
  the shared damage pipeline, use the FOLLOW_RANGE attribute, and keep moving
  toward a briefly hidden target for the vanilla five-tick grace period.
- `entity.armor_stand_death_event`: armor-stand kill paths now emit the
  `ENTITY_DIE` game event with the stand position and UUID before removal, so
  sculk listeners observe the same authoritative death boundary as vanilla.
  Damage tags now match `ArmorStand.hurtServer`: fire/campfire ignition,
  repeated 0.15 burn damage, four-point `on_fire` damage and the Mob-only
  `mob_griefing` guard are handled before breakability checks.
- `entity.painting_break_drop`: painting damage is now idempotent, emits the
  break sound and `ENTITY_DIE` event before removal, honors the `entity_drops`
  gamerule, and suppresses the item drop for creative player breaks.
- `entity.item_frame_break_drop`: item frames now remove a displayed item on
  the first non-explosive hit, preserve comparator updates and `BLOCK_CHANGE`,
  respect `Fixed`, `entity_drops`, creative players and item drop chance, and
  drop the correct normal/glow frame item when the empty frame breaks.
- `entity.end_crystal_damage`: end crystals now reject duplicate, invulnerable,
  and Ender-Dragon damage, emit death/explosion game events, perform the
  vanilla power-six explosion for non-explosion damage, and notify the active
  dragon-fight respawn state after removal.
- `world.item_scatter_rng`: container drops retain vanilla stack splitting and
  triangular velocity ranges, but their async-safe RNG stream is now derived
  from world seed, source position, world age and item identity rather than a
  process-global seed, making concurrent drops replay-stable.
- `item.tool_use_damage_pipeline`: axe, hoe, shovel and flint-and-steel use a
  shared Player durability path that preserves the original item identity for
  plugin events/statistics, emits the correct main/off-hand break status, and
  lets the network boundary commit the resulting snapshot once.
- `item.hoe_drop_merge`: hanging-root drops created by rooted-dirt tilling are
  inserted through the authoritative ItemEntity registry and immediately use
  the same entity-id-ordered merge path as periodic item ticks, preventing a
  duplicate visible item entity during burst drops.
- `mob.blaze_charged_metadata`: the Blaze attack sequence now owns a debounced
  charged flag and publishes it through the generated Blaze metadata slot, so
  client animation state follows the fireball volley and resets cleanly.
- `player.riptide_spin_pose`: successful Riptide launches now set a bounded
  auto-spin timer; pose selection uses `SpinAttack` for its lifetime, the
  timer decays at the player tick boundary without underflowing, and the
  launch gate accepts flowing/touching water or rain like `isInWaterOrRain`,
  rejects passengers and last-durability tridents before entering use, and
  emits the strength-specific vanilla Riptide sound and applies the grounded
  player's vanilla upward launch impulse.
- `player.flight_fall_damage`: flying players clear accumulated fall distance
  at the same pre-tick boundary as `Player.aiStep`; players with the vanilla
  `mayfly` ability never apply fall damage, including after leaving flight.
- `mob.enderman_carried_block_drop`: Endermen now expose the vanilla custom
  death-drop hook. A carried block is consumed atomically, evaluated through
  its block loot table with the death state/position/biome and a deterministic
  entity-specific seed, and is suppressed by the mob-loot gamerule without
  incorrectly applying the unrelated tile-drops gamerule.
  Successful Enderman pickups also emit `BLOCK_DESTROY` from the removed
  position with the Enderman UUID, after the authoritative block mutation.
- `lighting`: signed section bounds now return zero below the world and full
  sky light above the stored sections; active dimension profiles provide the
  min/height bounds, no-skylight dimensions stay at zero, and unloaded chunks
  return a diagnostic error from light setters.
- `persistence.chunk_unknown_nbt`: root-level chunk tags that Pumpkin does not
  yet interpret are retained and merged back on Anvil writes; block/biome
  palette, scheduled-tick and unknown-tag round-trip tests pass. Block and
  fluid scheduled ticks are filtered by owning chunk on load, matching
  vanilla's `SavedTick.filterTickListForChunk` boundary and preventing a
  malformed/stale region entry from scheduling work in a neighbour.
- `persistence.entity_region_unknown_nbt`: entity-region root metadata survives
  load/save; live block entities retain opaque fields from their original NBT,
  and unknown block-entity IDs are restored to pending chunk data instead of
  being dropped. Generic entity serialization now also retains unrecognised
  entity-root fields (for example `Leash`, `data` and future version fields)
  while live `Air`, `FallDistance`, `Silent`, `NoGravity` and `Glowing` values
  are authoritative on write. Explicit block-entity removal now clears both live
  and pending copies, preventing a broken block from resurrecting after reload.
- `persistence.proto_chunk_resume`: conversion through the generation
  `ProtoChunk` boundary preserves block/fluid scheduled ticks, pending block
  entities, `InhabitedTime` and forward-compatible root NBT.
- `datapack.zip_reload`: enabled directory and ZIP packs are validated
  transactionally; an explicitly enabled but missing `file/` pack now fails
  the candidate instead of clearing the active runtime. Recipes are overlaid
  by pack priority, and `/reload`
  publishes the new recipe snapshot only after a complete parse. Datapack tag
  resources now use the same loader: `replace`, optional entries, nested tag
  references, duplicate suppression, cycle detection, and known vanilla
  item/block/fluid existence checks are handled before publication. Resource
  functions (`data/<namespace>/function/*.mcfunction`) are loaded from both
  directory and ZIP packs, with BOM/comment normalization, bounded line and
  file sizes, deterministic later-pack override, and atomic snapshot
  publication. Function tags preserve declaration order and de-duplicate
  entries; required missing functions abort reload while optional entries are
  ignored. `/function` resolves canonical namespaced IDs, executes lines in
  order while preserving command-source context, continues after an individual
  line failure, supports `/function #namespace:tag` in declaration order, and
  refuses recursive depth beyond 64. `minecraft:load` runs
  after startup and successful reload, while `minecraft:tick` runs once before
  each normal world tick. Recipes, tags and functions are committed together
  under one runtime lock and generation counter. `/schedule function` now uses
  a trigger-time plus insertion-sequence queue with replace/append/clear,
  executes function or tag callbacks before the matching world tick, restores
  vanilla callback compounds from `data/minecraft/scheduled_events.dat`, and
  preserves unknown callback types on rewrite.
  Raw loot-table, predicate and advancement JSON, together with named or
  gzip-compressed structure NBT, is parsed into the same immutable datapack
  candidate. Resource IDs are canonicalized, later packs override earlier
  resources, malformed shapes/invalid NBT reject the whole candidate, and
  directory/ZIP loading share the same validation path. Deferred chests now
  resolve supported datapack item/empty/tag/nested-table entries with
  deterministic rolls and count functions, while recursive/unsupported tables
  are restored to their original LootTable NBT; the /loot loot command uses the
  same datapack-over-generated priority and seed. Typed runtime consumers for
  full loot conditions/functions/components, predicates, advancements and
  structures are still required for complete parity.
- `redstone.daylight_detector`: effective sky brightness, dimension sky
  darkening, rain/thunder attenuation and the vanilla sun-angle easing are
  implemented, with pure curve and weather-layer tests.
- `block.conduit_server_tick`: conduit frame validation, active/inactive
  transitions, wet-player Conduit Power, ambient timing, hostile target
  selection/attack and target NBT persistence now run from the block-entity
  tick. Target selection uses an explicit registry mapping for Java's `Enemy`
  marker (including Bogged, Breeze and Creaking), rather than the broader
  `MobCategory::MONSTER` approximation. Client rotation/particles and live
  water-frame fixtures remain pending.
- `fluid.mob_bucket`: player and dispenser bucket paths spawn fish, axolotl or
  tadpole entities after successful placement, instead of silently deleting
  the mob payload. The `minecraft:bucket_entity_data` component is decoded as
  custom NBT, variant components and the standard `NoAI`/`NoGravity`/`Silent`/
  `Glowing`/`Invulnerable`/`PersistenceRequired`/`Health` flags are applied
  before spawn, and protected position/UUID/type fields cannot overwrite
  server-owned values. Axolotl variants, salmon sizes and tropical-fish packed
  pattern/color variants now also publish their Java metadata and persist in
  entity NBT. Firework entities persist `Life`, `LifeTime` and their complete
  `FireworksItem` component stack across entity-region saves.
- `dispenser.equipment`: dispensable `Equippable` items now search the vanilla
  one-block target volume, honor the item entity allow-list/tag, equip the
  first compatible living target and consume the item only after success.
- `dispenser.shulker_and_shears`: shulker box items use directional block
  placement and preserve the item `container` component; shears shear the
  first eligible sheep or harvest a full beehive/nest, with correct durability
  consumption and fallback dispensing. Sheep wool drops use the unbiased
  vanilla `1..=3` random range rather than byte-modulo reduction.
- `dispenser.projectiles`: experience bottles now use a real projectile entity;
  the stack is consumed at launch and 3..=11 XP is spawned only at collision.
- `inventory.oversized_stack_arithmetic`: menu insertion and item-entity merge
  paths widen untrusted u8 counts before addition, so malformed 255-count
  stacks cannot wrap into an accepted duplicate or panic the server.
- `entity.experience_orb_lifecycle`: experience orbs now persist the vanilla
  `Value`, `Age` and merged `Count` fields, publish the value in the Java spawn
  data, perform the 20-tick same-value merge scan, follow the nearest eligible
  player within eight blocks, and use the vanilla water/lava movement rules.
  Awards first coalesce into an existing matching orb and reset its age, while
  pickup consumes one merged count at a time, so a merged orb cannot award XP
  repeatedly or disappear before its remaining value is collected.
- `player.underwater_mining_speed`: mining speed now checks the player's eye
  position against the actual water surface and applies the vanilla underwater
  penalty only when submerged; Aqua Affinity is honored exclusively from the
  head equipment slot, while the airborne penalty remains independently
  composable.
- `player.swimming_and_flying_travel`: Player swim-state entry now requires the
  vanilla water fluid at `blockPosition`, while an existing swim state only
  needs to remain in water. Player travel now applies the vanilla
  pitch-directed swimming assist (including the head-fluid guard) and preserves
  the pre-travel vertical input while ability flight uses the 0.6 multiplier.
- `entity.living_climbable_blocks`: `LivingEntity` now follows the vanilla
  climbability predicate for spectators, Elytra glide-through blocks, the
  `minecraft:climbable` tag, and open trapdoors above direction-matched
  ladders, while retaining the exact last-climbable block position.
- `entity.armor_helmet_damage_tag`: armor durability now honors the generated
  `minecraft:damages_helmet` damage-type tag, so falling anvils, blocks and
  stalactites damage only the helmet while ordinary damage still traverses all
  eligible armor slots.
- `entity.cross_dimension_transfer`: non-player entities now perform an actual
  world transfer during portal/teleport paths: source registries, spawn caps
  and watchers are cleared, the world pointer changes, and the destination
  registers and initializes the same live entity instead of leaving it ticking
  in the old dimension.
- `player.elytra_start_gate`: the Elytra start command now requires an airborne,
  unmounted player wearing a non-breaking Elytra in the chest slot; active
  fall-flying is cleared when those conditions stop being true.
- `player.sleeping_pos_persistence`: sleep state now uses the vanilla bed
  position rather than only a wake timer, persists `sleeping_pos` as the
  vanilla BlockPos `IntArray`, restores the sleeping pose after reload, and can wake safely even
  when respawn-point data is temporarily absent.
- `entity.elytra_flight_integrator`: fall-flying movement now follows the
  vanilla pitch/lift steering equations, Elytra drag, climbable-block stop,
  and kinetic `fly_into_wall` collision damage instead of the old ordinary-air
  TODO path.
- `entity.predicate.valid_inventories`: the inventory predicate now accepts
  only live chest/hopper minecarts, matching vanilla's `isAlive && Container`
  boundary instead of returning false for every entity.
- `dispenser.trigger_on_place`: a dispenser placed into an already powered
  position now mirrors `TRIGGERED` immediately and schedules the vanilla
  four-tick dispense edge, including power supplied from above.
- `dispenser.water_potion_and_golem_blocks`: ordinary water potions now convert
  vanilla mud-convertable blocks to mud and return a glass bottle to the
  dispenser inventory; carved pumpkins use the special snow/iron golem-base
  registration instead of being ejected as a generic block item.
- `recipe.crafter`: Crafter persistence now uses the vanilla `disabled_slots`
  int-array (with legacy scalar read compatibility), powered placement queues
  its four-tick edge, block/entity `TRIGGERED` mirrors stay synchronized, and
  inserting an item automatically re-enables a disabled empty slot.
- Dispenser entity, block, fluid, equipment, shear and projectile actions now
  emit their authoritative game events through the source `ItemStack`, so
  `minecraft:use_effects.interact_vibrations=false` suppresses only that
  action (including `PROJECTILE_SHOOT`) instead of leaking a vibration.
- `dispenser.spawn_egg_collision`: spawn eggs now use the centered target block
  and the entity collision volume before consuming an egg; blocked or occupied
  targets follow the ordinary failed-dispense eject path instead of creating a
  mob inside a wall or deleting the stack.
- `dispenser.respawn_anchor_charge`: glowstone now uses the same bounded charge
  operation for player and Dispenser paths; only Nether anchors below four
  charges consume the item and emit the charge sound.
- `crafter.lifecycle`: the six-tick crafting state is persisted and ticked by
  the block entity, comparator output counts disabled slots, and recipe output
  is emitted only after the atomic input transaction succeeds.
- `sculk.vibration_delivery`: Mojang frequencies, radius falloff, occlusion,
  wool dampening, travel-time queues, calibrated filtering, resonance and
  persisted pending state are implemented; sensors also force-schedule a
  stationary entity's `STEP` vibration, shriekers emit the vanilla `SHRIEK`
  game event and particle, carry source entities, and use the per-player
  warning tracker and spawn-rule gates.
- `persistence.entity_equipment`: living entities write the Java 26.2
  `equipment` compound and mobs write non-default `drop_chances`; readers also
  accept Pumpkin's former hand/armor compounds. Armor stands drop their
  stored equipment when broken.
- `blocks.farmland_dirt_path`: scheduled conversion ticks re-check support and
  hydration at execution time, so stale ticks cannot destroy a newly planted
  crop; dirt paths accept fence gates as vanilla does.
- `tick.fluid_inflight`: fluid scheduled ticks now remain observable while
  their async handlers run and are cleared only after the handler join.
- `redstone.scheduler_order`: the absolute-time scheduler now drains every
  due entry (`trigger_time <= current_time`), preserving priority/sub-tick
  order and allowing zero-delay callbacks to execute on the next tick instead
  of being stranded in an already-drained bucket; queued and in-flight work
  remain visible to `is_scheduled`.
- `world.iterative_neighbor_updates`: normal six-direction neighbour callbacks
  now use a FIFO collecting queue when nested callbacks request more updates;
  shape-state replacements from the block registry use a second FIFO worker,
  so `set_block_state` cannot recursively grow the async stack. Both queues
  retain the fixed update order and a 100,000-update cap reports and drops an
  abusive cascade instead of overflowing the process.
- `entity.tracking_chunk_barrier`: entity visibility now waits for a delivered
  target chunk, replays entities after delivery, and requeues cancelled or
  failed chunk sends. All generic chunk broadcasts now share that delivery
  barrier, so light/status/block-entity packets cannot arrive before the chunk
  or after its watcher state has been revoked. A per-player paired-ID set now
  suppresses duplicate spawn replays, removes actors before chunk forget/unload,
  and removes fast entities even after they cross a boundary. Movement,
  rotation, head-yaw, velocity and metadata deltas now use the same paired-ID
  gate, so a client cannot receive a delta before spawn or after removal.
  Java `ClientTickEnd` now tracks the last accepted client movement delta and
  resets it to zero when a tick contains no movement packet, matching
  `ServerGamePacketListenerImpl` and the player known-movement predicates.
- Bedrock `StartGame` now advertises member/operator permission from the
  player's actual configured operator level instead of hard-coding every
  connection as an operator.
- `world.tick_phase_order`: player, entity and block-entity ticks are now
  processed in deterministic order (players in connection order, entities by
  server entity id, block entities by block position), with chunk ticking
  completed before those phases. Each item still runs in an isolated spawned
  task so a panic is logged and does not abort the server, but no phase can
  observe another phase's partially committed state.
- Entity query helpers now stop at the requested maximum instead of appending
  one extra player/entity at the capacity boundary.
- `chunk.loading_ticket_recovery`: a stale derived chunk-level map no longer
  panics the server during ticket removal. The authoritative ticket set is
  rebuilt, affected stage transitions are published to the worker, and a
  duplicate-ticket regression test covers retaining the lowest remaining
  source level.
- Equipment deltas now use the same paired-ID and delivered-chunk barrier;
  Java armor/equipment packets are not sent to watchers that have not received
  the entity spawn, while Bedrock keeps its edition-specific equipment path.
  Attribute deltas now use the same barrier and still reach the owning player
  connection for self-health/effect updates. Entity swing, hurt and Java damage
  event packets use the same paired watcher set instead of world-wide leaks.
- `world.block_update_delivery`: batched multi-block updates use the same
  delivered-chunk barrier as single updates.
- `entity.effect_tracking`: authoritative mob-effect add/remove packets are
  restricted to delivered entity chunks; the add path preserves all client
  flags, including `blend`, instead of broadcasting effect IDs to every world
  player.
- `entity.sleep_mount_lifecycle`: entering a bed first dismounts the player
  through the vehicle's authoritative passenger removal path, preserving the
  passenger graph and teleport/movement packet ordering.
- `world.simulation_distance`: chunk-loading tickets use the platform-specific
  Java or Bedrock simulation distance.
- `spawning.natural_rng`: ordinary natural-spawn passes now derive one
  reproducible random stream from world seed, world age and chunk coordinates;
  group selection, water-ambient suppression, jitter, rotation and the random
  branches in monster/guardian/ghast/drowned/bat/slime/ocelot placement
  predicates no longer consume a process-global RNG while chunks are spawned
  concurrently. Thunder strike/skeleton-horse sampling and distance-despawn
  rolls likewise derive from world age and stable chunk/entity identity,
  retaining their vanilla probabilities without entity-tick races.
- `world.random_tick_sampling`: random-tick speed zero produces no samples;
  non-zero samples now use a reproducible per-world-age/per-chunk stream, so
  plant growth and other random-tick behavior no longer changes when active
  chunks are processed in a different order. The legacy Level API remains
  available for callers that do not provide a world-age seed.
- `weather.persistence_and_dimensions`: rain/thunder flags and timers round
  trip through vanilla `data/minecraft/weather.dat`; Nether, End and ceiling
  dimensions do not advance weather.
- `inventory.sided_transfer`: hopper and hopper-minecart transfers now carry
  the vanilla face direction into the destination/source inventory. Furnaces
  accept ingredients from above and fuel from the sides, expose result (and
  bucket remainder) below, brewing stands expose ingredient/bottle/fuel slots
  on their Mojang faces, and shulker boxes reject nested shulker boxes through
  every face. Storage minecarts use the same generic sided contract.
- Hopper entity pickup now matches `HopperBlockEntity.suckInItems`: a full
  collision block above the grid-aligned hopper blocks item entities unless it
  carries `does_not_block_hoppers`; non-full shapes and tagged transparent
  covers remain eligible.
- Brewing stand ingredient validation is derived from generated item/potion
  recipe tables, bottle slots accept only potion/glass-bottle stacks, and the
  hopper merge hook prevents inserting into occupied bottle slots.
- Container-menu lifecycle now opens and closes furnace/brewing-stand
  inventories explicitly. Stonecutters also clear their virtual output and
  return the real input stack on menu removal, preventing item loss and stale
  preview duplication after disconnect/reopen.
- `fluid.waterlogging`: flowing water fills dry waterloggable blocks while
  preserving every non-fluid property; double slabs and lava are excluded and
  the fluid tick is scheduled after the replacement.
- `redstone.lightning_rod`: placement uses the clicked outward face, carries
  source-water logging, schedules water ticks, emits strong power only outward,
  and resets powered state after the vanilla eight-game-tick pulse.
- `redstone.comparator_feedback`: switching compare/subtract mode now emits
  the vanilla comparator click with the mode-specific pitch after the state
  mutation succeeds.
- `respawn.cross_dimension`: respawn-point validation resolves the stored
  dimension world and loads its target chunk before reading a bed/anchor; the
  death world's blocks can no longer invalidate a valid cross-dimension bed.
- `player.respawn_client_state_sync`: respawn now explicitly re-sends retained
  experience after `CRespawn` when `keepInventory` leaves the server-side XP
  unchanged; the reset path clears effects, fire, velocity and fall state before
  the new dimension's world-info and center-chunk packets.
- `player.previous_gamemode_persistence`: successful gamemode transitions now
  persist the immediately preceding mode as `previousPlayerGameType`, while
  cancelled and no-op changes leave it untouched.
- `entity.fall_death_message_variants`: fall deaths now use the last recorded
  climbable block to select vanilla ladder/vine/scaffolding/other-climbable
  messages, distinguish weeping and twisting vines on Java, and use Bedrock's
  water-specific translation when the entity fell from water; water detection
  follows the fluid tag so flowing and waterlogged water are not missed.
- `inventory.click_safety`: stale or malicious container slot packets are
  rejected and resynchronized before indexing the menu; received sync stacks
  also validate their slot bounds. Property updates now reject the exact
  `len` boundary and invalid negative slots instead of indexing/panicking.
- `inventory.menu_distance`: Java slot clicks for block-backed menus now apply
  the vanilla eye-to-block-AABB `stillValid` range (`blockInteractionRange` plus
  the four-block menu buffer) and the original block-id check captured at open
  time. Rejected interactions resynchronize instead of mutating a remotely
  accessed/replaced container; direct plugin menus remain positionless and
  unaffected.
- `player.hunger_bounds`: food/effect additions saturate at the vanilla
  20-point limit; saturation, exhaustion and loaded food NBT are finite and
  bounded, so high-value datapack effects cannot wrap a `u8` hunger counter or
  inject invalid floating-point state.
- `inventory.number_key_swap`: number-key swaps exchange two occupied stacks,
  split oversized hotbar stacks while writing the remainder back to the source
  slot, and reject self-slot no-ops without changing the transaction.
- Player-menu armor and off-hand clicks/shift-clicks now publish the final
  entity-equipment stack using the authoritative menu slot (5..8/45), even
  when an item has no or an incorrect `Equippable` component.
- Shield blocking now maps `Hand::Right`/`Hand::Left` to main/off-hand
  correctly, applies durability to the active shield, sends non-breaking
  durability updates to the client, and emits break status/sound/statistics
  without retaining the active-hand mutex.
- Sign JSON now accepts vanilla camel-case `clickEvent`/`hoverEvent` fields;
  a clicked sign line executes root `run_command` events through the player's
  permission-aware command source, while malformed/legacy text keeps the
  ordinary edit path.
- `inventory.drag_transactions`: quick-craft drag admission follows the vanilla
  selected-slot/count rule and its start/select/end state machine, malformed
  interleaved clicks reset without mutating slots, duplicate selections are
  ignored, left drags distribute the exact remainder, right and creative drags
  respect per-slot/component stack limits, and double-click pickup-all follows
  the vanilla direction and partial-before-full pass order.
- `spawning.distance_and_danger`: natural spawning uses the configured world
  spawn and active chunk set instead of the origin/start-chunk approximation,
  and rejects fire, soul fire, berry bushes, wither roses and cacti as empty
  spawn volumes.
- `worldgen.dimension_sea_level`: `GenerationCache` now carries the owning
  generator's sea level. Blue ice, icebergs, basalt columns and freeze-top-layer
  use that value, so Nether/custom generator settings no longer inherit the
  Overworld y=63 assumption; an Overworld/Nether proto-chunk regression test
  verifies 63/32 propagation.
- `worldgen.simple_block_feature`: the feature now handles pale moss carpet's
  vanilla base/wall state derivation and randomized non-base topper, including
  support-face checks, lower-layer restrictions and LOW→TALL side promotion.
- `protocol.java_light_updates`: live Java light updates now share the exact
  mask/array writer with initial chunk packets. The serializer emits the
  synthetic below/above-world sections, supports dimensions wider than one
  `i64` mask word, and preserves vanilla low-nibble-first `DataLayer` bytes.
- `worldgen.coral_direction_rng`: coral claw/tree branches now use the exact
  vanilla horizontal direction lists and Fisher–Yates draw order; coral claws
  also choose the side/up segment with the vanilla two-way RNG and lengths.
  Coral mushroom placement now uses the original origin-relative coordinates,
  sink offset, dimensions and corner/shell predicate instead of the previous
  doubled-origin approximation.
- `worldgen.attached_logs_decorator`: tree attached-log decorators now consume
  RNG in vanilla order: they process a Fisher–Yates shuffled copy of the log
  positions, choose a fresh configured direction for each position, then apply
  the probability and air checks. Empty direction sets are safe no-ops; the
  deterministic permutation and direction-set boundaries have unit tests.
- `worldgen.twisting_vines_feature`: Nether twisting-vine generation now uses
  inclusive vanilla spread offsets, the 1/6 double-height and 1/5 single-height
  gates, the exact netherrack/warped support set, and a generated head state
  with `AGE` 17..25. The plant/head state boundary and all valid head ages are
  covered by worldgen tests.
- `worldgen.weeping_vines_feature`: roof weeping vines now reproduce the two
  vanilla patch passes (200 nether-wart and 100 vine attempts), exact
  difference-of-uniform offset ranges, one-neighbour roof-wart rule, height
  gates and age-bearing head/plant column states. Support and age round-trip
  boundaries are tested.
- `worldgen.place_on_ground_decorator`: tree ground decorations now use the
  vanilla solid-render/non-leaf support predicate and
  `MOTION_BLOCKING_NO_LEAVES` heightmap clearance in addition to the existing
  bounding-box sampling.
- `worldgen.vines_feature`: generated vines now use the neighbour's full
  support/collision face (the `MultifaceBlock.canAttachTo` contract) instead
  of the over-restrictive full-cube flag, while preserving vanilla direction
  order and face-state construction.
- `worldgen.cocoa_decorator`: jungle-tree cocoa now preserves the configured
  probability through codegen, applies the one-time gate and per-side 25%
  attempts in vanilla order, and writes age 0..2 with outward-facing states.
  The generated jungle-tree configuration and state round-trip are covered by
  focused tests.
- `worldgen.state_providers`: randomized-int and rotated/pillar providers now
  actually sample their configured provider/RNG and rewrite only the selected
  state property, retaining every other property and failing closed for
  malformed custom states.
- `worldgen.attached_leaves_decorator`: foliage decorators now preserve the
  full vanilla shuffled-position/direction/probability order, required-air
  corridor checks and inclusive exclusion blacklist. Codegen now retains the
  complete configured provider/radii/direction fields instead of constructing a
  no-op unit decorator.
- `worldgen.creaking_heart_decorator`: pale-oak trees now use the configured
  probability, shuffled log candidates and the six-neighbour `#minecraft:logs`
  predicate before placing the vertical natural/dormant heart state; codegen
  preserves the configured probability instead of emitting a unit no-op.
- `worldgen.beehive_decorator`: bee trees now choose the vanilla hive height,
  shuffle the allowed horizontal candidates, require an air exit, place a
  south-facing honey-level-zero bee nest and attach two or three modern typed
  bee occupants. The block entity loader accepts both modern lowercase `bees`
  and legacy uppercase `Bees` data.
- `worldgen.pale_moss_decorator`: pale-oak trees now preserve all three
  configured probabilities, shuffle logs before choosing the lowest origin,
  invoke the registered pale-moss patch for the ground pass and place exact
  tip/non-tip hanging-moss states below trunks and foliage.
- `worldgen.alter_ground_decorator`: mega spruce/pine ground alteration now
  uses the lowest trunk/root layer, vanilla fixed and random circle offsets,
  corner exclusion and context-aware optional state providers instead of a
  no-op decorator.
- `worldgen.simple_block_feature`: simple generated blocks now use the
  context-aware placement predicate and persist the configured one-tick normal
  block tick in proto-chunk data when `schedule_tick` is enabled; double plants
  are now placed as an atomic lower/upper pair like `DoublePlantBlock.placeAt`.
- `worldgen.pointed_dripstone_feature`: pointed-dripstone generation now uses
  the vanilla up/down direction gate, patch spread draws, taller-column check,
  and base/middle/frustum/tip state sequence with waterlogging retained.
- `spawning.bat`: bat natural-spawn checks now match vanilla's half-probability
  gate and test the `bats_spawnable_on` tag on the block below the spawn point;
  roosting state is also sent through the Java entity metadata flag.
- `spawning.shared_predicates`: common animal, surface-water, axolotl and
  tagged frog/mooshroom/rabbit/wolf predicates now use vanilla support tags,
  sea-level bounds and raw-brightness thresholds; armadillo/camel/fox/goat/
  parrot support tags, polar-bear alternate-biome rules, turtle sand bounds,
  lush-cave tropical-fish height rules, glow-squid darkness/depth and the
  ocelot one-in-three gate are also covered. Thunder monster brightness uses
  the dimension-aware raw-light API.
- `spawning.drowned`: natural drowned spawning now has its own two-water-cell,
  dark-light, deep-water and biome-dependent 1/40 or 1/15 chance predicate.
- `spawning.spawn_placements`: blaze/breeze/zoglin now use the any-light
  monster predicate; husk, camel husk and parched use the surface-monster
  sky-visible predicate; strays account for powder-snow obstruction; surface
  slimes use the biome tag, Y range, moon-phase chance table and light gate.
  On-ground placement now also applies the default vanilla support predicate
  (upward sturdy face plus light emission below 14), and LAND adjustment uses
  the generated collision shape rather than the coarser full-cube flag. Both
  runtime and chunk-generation candidates are checked against the current
  WorldBorder before the placement predicate runs.
- `entity.spider_climbing`: spiders and cave spiders set climbing from
  horizontal collision before movement resolves, keep the flag for the same
  tick's wall motion, and publish the Java `SPIDER_FLAGS` metadata when the
  target protocol version exposes it.
- `entity.cave_spider_poison`: the common melee path now exposes a successful
  attack hook; cave spiders apply Poison for 7 seconds on Normal and 15
  seconds on Hard, never on blocked/cancelled hits or Easy/Peaceful.
- `fluid.sponge_absorption`: sponge BFS now follows vanilla depth/count bounds,
  removes water, bubble columns, kelp and seagrass variants, retries on every
  neighbor update, and routes plant removal through normal drops/block events.
- `entity.firework_components`: firework rockets retain the `fireworks` item
  component, use the vanilla flight-duration lifetime formula, and publish the
  Java `ID_FIREWORKS_ITEM` metadata for dispenser, hand-launched and placed
  rockets. Hand-launched elytra rockets now pass the exact held stack into the
  entity, so custom flight duration and explosion payloads are not discarded.
  Rockets with explosion components also apply the vanilla visible target
  damage formula within five blocks.
- Firework use-on-block now follows the 26.2 click-position plus face-offset
  spawn formula and defers to the elytra hand-use path while fall-flying.
- `item.use_remainder`: generated item data decodes vanilla `UseRemainder`
  templates; exhausting stew, milk, potion and honey-bottle stacks uses that
  component path with creative/non-final-stack semantics.
- `item.honeycomb_waxables`: Honeycomb now covers every generated 26.2 copper
  collection (bars, chains, chests, golem statues, lanterns and lightning rods)
  across unwaxed oxidation stages, preserves block-state properties and consumes
  one item on successful use; mapping boundaries have a regression test.
- `item.name_tag_validation`: Name-tag use now follows vanilla's live
  serializable-living-target gate, sets `PersistenceRequired` on mobs and never
  consumes an item on invalid/dead or nameless targets. Mob distance-despawn
  now runs before AI and protects persistent, named, and leashed entities;
  category distance and 1/800 random-gate tests pass. Real multi-player
  despawn fixtures and subtype-specific persistence overrides remain.
- Natural-spawn cap accounting now excludes `PersistenceRequired` mobs and uses
  saturating decrements, preventing a name-tag persistence transition from
  suppressing or underflowing unrelated spawn categories.
- `lighting.changed_sections`: runtime writes mark only sections whose nibble
  value actually changed; live Java light updates filter their masks and arrays
  to that set while initial chunk packets remain complete.
- `entity.effect_lifecycle`: repeated effects follow the vanilla
  stronger/longer replacement rule; weaker or shorter applications leave the
  live instance untouched, missing-effect removals are no-ops, and effect
  packets retain all client flags while respecting delivered-chunk tracking.
  Stronger short-lived effects retain a recursive hidden-effect chain and
  restore it on expiry; the chain is included in entity NBT round trips.
- `protocol.varint_overflow`: Java/Bedrock VarInt, VarUInt, VarLong and
  VarULong decoders reject payload bits outside the final representable byte
  instead of silently truncating malformed packets.
- `redstone.target_projectiles`: target blocks receive all common projectile
  impacts through a world-level hook, calculate vanilla face-plane strength,
  emit weak and strong power, schedule every `AbstractArrow` impact (including
  tridents) for 20 ticks and other projectiles for 8 ticks, and count
  arrow/trident target hits
  in the custom statistic. The pure strength and activation mapping have
  boundary tests.
- `block.campfire_projectile_ignition`: the block-side projectile hook matches
  vanilla's fire/lit/waterlogged gate and updates the full campfire state with
  listener/neighbor notifications. Player-owned projectiles now also honor the
  owner's block interaction range, matching the server-side `mayInteract` gate.
- `block.mushroom_survival_light`: mushroom placement and neighbor survival now
  require a solid-render support block plus raw brightness below 13, while the
  vanilla override tag bypasses the light limit. Boundary behavior is covered
  by a unit test; generated-world light fixture remains pending.
- `entity.minecart_subtypes`: command-block and spawner minecarts now have
  dedicated state instead of falling through `MinecartKind::Other`. Command
  carts execute on powered activator rails with the vanilla four-tick cooldown,
  persist command/output/success-count NBT, expose command success as detector
  comparator output, publish command/spawner display block metadata, and enforce
  level-2 interaction;
  spawner carts reuse the weighted SpawnPotentials/player-range/nearby-cap
  state machine at their moving block position. Bedrock editor/UI, entity
  events and exact client fixtures remain explicitly pending. Rail movement now
  uses the common `Entity::set_pos` path, so `pos`, `block_pos` and `chunk_pos`
  cannot diverge; unload and tracker cleanup also derive the destination chunk
  from the live block position. Projectile constructors use the same path.
- `world.explosion_fire`: explosion sources now carry an explicit incendiary
  flag. Fireballs use the vanilla one-in-three placement pass after block
  destruction; every candidate must still be air and pass the normal FireBlock
  survival predicate, while TNT, mobs, beds and anchors retain the ordinary
  non-incendiary path until their source-specific Java rules are wired.
- `command.summon_nbt`: `/summon` now accepts a compound SNBT argument with
  nested lists and embedded spaces. The parsed tag is loaded through the
  entity NBT interface before spawn registration, so position, motion, custom
  name and persistence flags are applied without bypassing entity validation.
- `command.worldborder_dimension`: every `/worldborder` subcommand now resolves
  the executor's own dimension when the sender is a player or command block;
  console/RCON/dummy senders retain the vanilla-compatible overworld fallback.
  Commands fail cleanly when no world is loaded instead of panicking, and all
  border mutations (diameter, center, damage and warning settings) use the same
  resolved world.
- `player.shared_spawn_finder`: initial Java/Bedrock login and fallback respawn
  now search the configured `respawn_radius` square (capped at 1024 columns),
  load each candidate chunk, reject fluid/ocean columns and incomplete support
  faces, enforce the world border and the complete player collision box, then
  apply a bounded vanilla-style height fixup when no candidate is valid.
- `item.boat_exact_raycast`: the shared world raycast now exposes the exact
  AABB entry point and hit face while retaining the legacy block-only API.
  Boat/raft placement consumes that point for `BoatItem.use`, so side hits,
  water surfaces and partial collision shapes no longer collapse to a guessed
  block centre.
- `player.bed_respawn_offsets`: bed respawn candidate selection now uses the
  player's yaw and the exact twelve-offset `BedBlock.findStandUpPosition`
  order, while retaining the shared collision and support checks for each
  candidate. Exact dangerous-block/passenger-shape parity in the shared
  respawn validator remains tracked separately.
- `block.bed_wake_villager`: an occupied bed now searches only the head-half
  volume, wakes a sleeping villager through the common entity hook, and clears
  the Java sleeping-position metadata before returning success. Other entities
  and villagers outside that block keep the vanilla occupied response.
- `spawning.collision_shape_empty_block`: natural spawning now rejects a
  position when its actual generated collision shape is a full cube, matching
  `NaturalSpawner.isValidEmptySpawnBlock`; it no longer treats the unrelated
  render/full-cube flag as the sole collision test. Fluid, dangerous-block,
  and prevent-inside filters remain enforced.
- `redstone.piston_same_tick_retraction`: moving piston block entities now
  record the current game tick before advancing progress. Retraction checks
  reproduce `PistonBaseBlock`'s same-tick/final-tick event selection at
  progress `>= 0.5`, including the server handling-tick phase, while the
  marker remains runtime-only as in vanilla NBT. Honey-block rider AABBs now
  use the progress at the start of the tick, matching `moveStuckEntities`
  before the half-step delta is committed.
- `worldgen.root_system_water_space`: root-system tree columns now distinguish
  air from water and apply vanilla's one-based water-height allowance; hanging
  roots require a sturdy support face instead of merely a non-air block.
- `worldgen.sapling_growth_gate`: ordinary saplings now use the vanilla
  one-in-seven random-tick gate before advancing from stage zero; bone-meal
  advancement remains an independent path. Stage-one configured tree
  generation and species-specific mega-tree selection are still open.
- `entity.mob_spawner_timer`: mob spawners now execute at zero delay without an
  extra sentinel tick, tolerate legacy negative delays, gate on live players,
  enforce nearby-entity limits using vanilla's inflated block AABB and validate/clamp malformed NBT ranges before
  spawning. Weighted `SpawnPotentials` entries and their complete entity
  compounds now survive load/save and are selected on each successful cycle;
  next delays use Mojang's half-open `minSpawnDelay..maxSpawnDelay` interval;
  each requested mob now gets three integer X/Z attempts, a one-block
  `ON_TOP_OF_COLLIDER` search, world-border validation, entity placement rules
  and collision checks;
  display-entity/subtype/client-fixture parity remains explicitly tracked.
- `entity.arrow_damage_parameter`: the arrow base-damage setter now mutates the
  value consumed by the vanilla-style hit damage formula instead of silently
  discarding plugin/enchantment changes.
- `entity.mob_experience_reward`: mob death rewards now use the asynchronous
  virtual reward path, preserve the baby/zero-reward gates, and add the
  vanilla one-to-three XP bonus for each eligible non-saddle equipment slot
  using a snapshot of equipment and drop chances. Killer enchantment bonuses,
  orb merge timing and full differential death fixtures remain.
- `entity.fishing_retrieval_damage`: fishing-hook retrieval now follows the
  vanilla durability table (3 for an item entity, 5 for another hooked entity,
  1 for a loot catch, 2 when stuck in ground), with on-ground precedence and
  active-hand damage for Java use. Caught items now use deterministic
  fish/junk/treasure pools, a 5×5×4 open-water check, and the shared supported
  datapack override path; full luck/enchantment loot functions, XP orb spawning and
  all bobber timing/collision edge cases remain.
- `protocol.max_damage_component`: the generated `minecraft:max_damage` item
  component now has a bounded VarInt encode/decode path instead of falling
  through the unimplemented component branch; negative wire values are
  rejected and the codec has round-trip/malformed tests.
- `protocol.item_component_codecs`: Java item-stack component patches now decode
  custom-data compounds (including Java CESU-8 strings), entity variant string
  components, numeric potion/dyed/ominous values and unit components instead of
  disconnecting through the generic TODO branch. Custom names now consume the
  Java `ComponentSerialization` NBT stream, oversized item counts/component IDs
  are rejected before narrowing, and length-prefixed component payloads are
  capped before allocation. Round-trip tests cover custom NBT, names, variant
  payloads, consumable apply-effects ordering, bounded consumable counts,
  ordinary item stacks and bundle templates; both serializers now follow the
  unprefixed vanilla stream codec.
- `protocol.use_effects_component`: `minecraft:use_effects` now has the
  vanilla three-field data model, default-aware persistent NBT encoding and
  the BOOL/BOOL/FLOAT network stream codec; generated item data references a
  shared default value and malformed speed multipliers are rejected. Active
  item use now applies the multiplier to air/water/lava movement and clears
  sprinting when `can_sprint` is false. Authoritative block and boat placement
  pass the used item through the vibration dispatcher, so absent components
  remain enabled and `interact_vibrations=false` suppresses only that action;
  every Dispenser projectile now emits `PROJECTILE_SHOOT` through the consumed
  source stack, so its vibration opt-out is honored too. Projectile collision
  events and consumables still need the same source-stack audit.
- `protocol.item_use_cooldown_gate`: Java use-item packets now stop at the
  server cooldown boundary before interaction hooks or item behaviours run;
  the registry-key fallback and component-defined cooldown group match the
  Bedrock path, preventing a client from bypassing an active use cooldown.
- `protocol.entity_interact_hand_and_cooldown`: Java `Interact` and
  `InteractAt` now validate and resolve the packet's main/off-hand value,
  reject invalid hands, enforce the resolved stack's use cooldown before
  entity/plugin callbacks, and write the mutated stack back to that same hand.
- `block.beehive_release_occupants`: dispenser shears now reset a full hive
  and release its stored bees through the hive's facing side. Modern wrapped
  `entity_data` and legacy direct compounds are supported; malformed or future
  occupants remain persisted rather than being silently lost. Hive shearing
  is evaluated before entity shearing, matching `ShearsDispenseItemBehavior`.
- `block.beehive_server_tick`: active block-entity ticking now advances
  `ticks_in_hive` with the strict vanilla 600/2400 boundary, retries blocked
  exits, restores saved `flower_pos`, preserves unknown occupant fields, emits
  work/exit sounds, and applies nectar delivery to the hive honey level.
- `block.dispenser_wither_skull_placement`: wither skulls now use the special
  placement path only when the candidate has a valid soul-sand/soil base and
  the dimension/difficulty/build-limit gates allow a wither; they preserve
  dispenser-facing rotation, invoke the normal wither-pattern callback, and
  retain fallback on blocked or base-less targets.
- `block.dispenser_optional_order_and_tnt_rule`: carved pumpkins now try
  golem assembly before generic equipment, and `tnt_explodes=false` follows
  vanilla's failed optional-dispense/eject path without priming TNT.
- `entity.sulfur_cube_runtime`: sulfur cubes now use a dedicated runtime
  wrapper around the authoritative cube movement/collision/NBT path, so a
  bucket spawn or chunk reload no longer falls back to generic LivingEntity.
- `item.sulfur_cube_bucket`: the sulfur-cube bucket is registered as a filled
  mob bucket, does not create an accidental water source (its vanilla content
  is `Fluids.EMPTY`), and spawns the stored entity for player and dispenser use.
- `block.dispenser_glass_bottle_sources`: glass bottles now collect any water
  fluid state without deleting a source, leave water cauldrons untouched, and
  release full-hive occupants before returning a honey bottle, matching the
  registered vanilla `DispenseItemBehavior` order.
- `world.saved_custom_bossbars`: `custom_boss_events.dat` is now part of the
  server lifecycle. Startup restores live bars (title, health, style, flags,
  visibility and player UUIDs); shutdown writes them back in vanilla's
  `data` map, regenerating runtime UUIDs and retaining unknown future entries.
  A gzip-NBT round-trip fixture covers removal, flags, player lists and root
  metadata preservation.
- `fluid.fire_near_rain_columns`: fire now uses vanilla's five-column
  rain probe (the fire cell plus west/east/north/south) for both extinguishing
  and spread suppression; vertical neighbours are intentionally excluded.
- `spawning.shared_predicates`: natural spawning now rejects generated
  redstone signal-source states before collision/fluid checks, matching
  `NaturalSpawner.isValidEmptySpawnBlock`; block spawners also pause without
  resetting their delay when `spawner_blocks_work` is disabled. Chunk-generation
  spawning now also obeys `doMobSpawning`, and jittered natural-spawn groups
  allow the current chunk while requiring the active-chunk gate only after a
  candidate crosses into a neighbour, matching `canSpawnEntitiesInChunk`.
  Pandas are explicitly dispatched through the registered
  `Animal.checkAnimalSpawnRules` predicate; their `NO_RESTRICTIONS` placement
  type must not fall through the conservative unrestricted-mob rejection.
  Chunk-generation group selection, counts, jitter and rotation now consume a
  deterministic world-seed/chunk-local population stream rather than the
  process-global RNG, so generated chunks no longer depend on thread timing or
  server restart order. Local mob-cap membership is recomputed from current
  player positions, and spawn-cost energy uses the rough biome at the actual
  candidate/entity position rather than a stale cached entity biome.
- `block.mushroom_random_spread`: brown/red mushrooms now use the vanilla
  1/25 random-tick gate, five-mushroom cluster cap, sequential four-offset
  search, and the exact light/support survival predicate before placement.
- `loot.deterministic_rng`: loot evaluation accepts an explicit seed and routes
  rolls, random conditions, tag expansion, number providers, bonus functions
  and nested tables through one deterministic RNG stream; guardian/elder
  guardian nested `gameplay/fishing/fish` references now use the fish-only
  weighted pool; count functions
  clamp/saturate instead of wrapping. LocationCheck compares the context biome
  for zero-offset checks and resolves non-zero offsets through an authoritative
  world-backed resolver (while contexts without one still fail closed). Runtime
  block, explosion, entity and command call-sites derive their seed from world
  seed, source position, game time and a source salt; deferred container loot
  retains its persisted seed. Seeded table and offset-resolution regression
  tests prove identical output and coordinate semantics.
- Block experience now uses the shared Silk Touch gate in `drop_loot`, so player,
  explosion and automatic destruction paths do not award XP through a Silk
  Touch tool. Its amount roll uses an independent fixed salt derived from the
  same loot-context seed, so replaying a seeded block break is deterministic;
  both properties have regression tests.
- Melee AI target filtering now keeps ordinary/survival/adventure targets and
  rejects creative or spectator players; the polarity is covered by a pure
  truth-table regression test.
- Chiseled bookshelf insertion now awards the vanilla `ITEM_USED` statistic
  for the inserted book exactly once at the successful mutation boundary;
  failed insertions and removals remain side-effect free.
- Chest and ender-chest opening now applies the vanilla cat occlusion rule:
  only a live sitting cat whose hitbox overlaps the block volume above the
  container blocks access; adjacent or dead entities do not.
- Plant survival now resolves fluids through the shared `BlockAccessor` API:
  lily pads use fluid support tags and require an empty fluid state above, while
  sugar cane accepts either adjacent fluid or block support tags, including
  waterlogged states.
- Sea-pickle bonemeal now probes the complete vanilla 1-3-5-3-1 diamond instead
  of the previous five-position line, preserving the one-in-six placement gate
  and four-pickle source mutation.
- Natural spawning now has the 26.2 Nautilus predicate: the sea-level band and
  exact water-below/Water-block-above checks are applied before generic water
  fallback handling.
- Chunk-generation mob population now reuses the live spawn-rule and collision
  gates, so generated creatures cannot bypass difficulty/light predicates or
  appear inside a stale heightmap collision volume.
- Big dripleaf projectile hits now force `FULL` tilt, play the vanilla tilt-down
  sound, and schedule the 100-tick recovery instead of being ignored.
- Tripwire scheduled checks now honor `Entity#isIgnoringBlockTriggers`: removed
  and spectator entities, bats, and marker armor stands no longer keep a wire
  powered, while ordinary entities and players still do.
- Infested blocks now mirror `InfestedBlock.spawnAfterBreak`: the captured
  breaking tool is available to block callbacks, and survival infestation is
  suppressed by `doTileDrops` and Silk Touch.
- Piston-head and moving-piston cleanup now passes the initiating player into
  the matching base break operation, retaining vanilla block-break ownership
  while still suppressing duplicate drops.
- Water spawn placement and chest/ender-chest occlusion now use the exact
  generated collision-shape-full-block predicate that backs vanilla
  `isRedstoneConductor`, consistently in runtime and generation-cache paths.
- Falling-block conversion now leaves the legacy fluid block prescribed by
  vanilla (`waterlogged -> water`, lava fluid -> lava) instead of always
  replacing the source with air. The carried state is also normalized to
  `waterlogged=false`, so the landing path cannot recreate a second fluid
  source from the original block's property.
- Piercing arrows now track transient already-hit entity IDs, enforce the
  vanilla `pierce + 1` distinct-target limit, and clear the per-tick collision
  guard so the arrow can continue through subsequent entities.
- Ender Chest placement now preserves a water source in its generated
  `waterlogged` property while retaining the vanilla facing.
- `parity/manifest.toml` and `tools/parity_inventory.py` provide a
  machine-readable ledger and deterministic source/TODO inventory. The
  inventory report currently validates 151 tracked contracts; all remain
  explicitly `mostly` until their world/packet/persistence boundaries have
  differential evidence.

## Verification

```text
cargo fmt --all -- --check
cargo check -p pumpkin --lib
cargo check --workspace
cargo test -p pumpkin --lib                         # 386 passed in the last full run
  cargo test -p pumpkin --lib block::entities::chest::tests::deferred_loot_table_can_be_restored_without_losing_seed  # 1 passed
  cargo test -p pumpkin-inventory --lib               # 13 passed
  cargo test -p pumpkin-data --lib use_remainder_converts_only_the_exhausted_survival_stack
  cargo test -p pumpkin-world --lib                   # 226 passed in the current checkout
  cargo test -p pumpkin-protocol --lib                # 93 passed
  cargo test -p pumpkin-protocol --lib light_update  # 4 passed
python3 tools/parity_inventory.py --output report.json
git diff --check
```

The Pumpkin library suite is green after the recipe-component,
firework, changed-section lighting, ProtoChunk-resume, bed-offset,
collision-shape, piston same-tick, root-system water-space, five-column fire
rain, signal-source spawn-filter, target-projectile-window, and honey-piston
movement additions, plus the player/dispenser brush state machine and the
  deterministic loot RNG stream and biome-aware LocationCheck context, plus
  transactional raw datapack resource loading for loot tables, predicates,
  advancements and structures.
The same run includes datapack function ZIP/directory priority, validation,
canonical `/function` and `/schedule` command coverage; the Pumpkin library
The last full Pumpkin suite is 386/386, including the deferred-chest and
storage-minecart datapack regressions, fishing pool/open-water coverage, and
minecart persistence-related regressions. World-info scheduled callback persistence has a
dedicated round-trip test in the world crate.
Pumpkin World baseline is green at 226/226, and the post-change chunk-loading
targeted suite is green at 2/2,
the protocol crate is 93/93 (including item-component and malformed-packet
guards), Java light serialization is 4/4, and inventory is 13/13. A complete
`cargo test --workspace --no-fail-fast` baseline run passed before the final
chunk-ticket recovery change; after that change the focused
`chunk_system::chunk_loading` suite passes 2/2. `cargo check --workspace`
also passes after the current changes. A complete `cargo test --workspace
--no-fail-fast` rerun remains intentionally separate because it previously
hit the host's disk-full condition; no test failure was observed in that run.
Formatting, diff check, and parity inventory pass.
These are repository-level gates; a release-candidate claim still requires the
real-client, persistence-fixture, and differential checks listed below.

## Remaining release blockers

The repository is not yet vanilla-complete. The remaining blockers are
tracked explicitly in the full plan and ledger: full typed runtime consumers
and gameplay reload semantics for datapack loot/predicates/advancements/structures
(raw snapshot and the supported deferred-chest/storage-minecart loot subset are now present),
complete Anvil block-entity/tick persistence
through live unload/reload and real-world fixtures,
full per-player tracker ACK/barrier semantics (the chunk-delivery boundary is
now guarded, but a complete `ChunkMap.TrackedEntity` delta tracker is still
needed), cross-dimension/Bedrock lighting packet fixtures, all dispenser behaviors
(bottle/entity bucket edge cases, exact mob/equipment persistence), calibrated sensor
frequency event propagation for every source, complete Warden spawn placement
rules, fluid/weather edge cases, mob AI/spawning, command/loot/advancement
coverage, generated protocol certification, Bedrock parity and the differential
Java 26.2 harness. These must be closed and promoted to `complete` before a
release claim.

The current ledger intentionally reports all 151 contracts as `mostly`: this is
not a build failure, but a release gate. Each row still needs its world/packet/
persistence boundary fixture and differential evidence before it may become
`complete`.
