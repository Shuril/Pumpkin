use super::BlockEntity;
use pumpkin_data::block_properties::BlockProperties;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::recipes::{CookingRecipeKind, get_cooking_recipe_with_ingredient};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::position::BlockPos;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

pub struct CampfireBlockEntity {
    pub position: BlockPos,
    pub items: [Arc<Mutex<ItemStack>>; 4],
    pub cooking_times: [tokio::sync::Mutex<i32>; 4],
    pub cooking_total_times: [tokio::sync::Mutex<i32>; 4],
    pub dirty: AtomicBool,
}

impl BlockEntity for CampfireBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let mut items = std::array::from_fn(|_| Arc::new(Mutex::new(ItemStack::EMPTY.clone())));
        if let Some(list) = nbt.get_list("Items") {
            for tag in list {
                if let Some(compound) = tag.extract_compound() {
                    let slot = compound.get_byte("Slot").unwrap_or(0) as usize;
                    if slot < 4
                        && let Some(stack) = ItemStack::read_item_stack(compound)
                    {
                        items[slot] = Arc::new(Mutex::new(stack));
                    }
                }
            }
        }
        let mut cooking_times = [Mutex::new(0), Mutex::new(0), Mutex::new(0), Mutex::new(0)];
        if let Some(arr) = nbt.get_int_array("CookingTimes") {
            for (i, &val) in arr.iter().enumerate().take(4) {
                cooking_times[i] = Mutex::new(val);
            }
        }
        let mut cooking_total_times = [Mutex::new(0), Mutex::new(0), Mutex::new(0), Mutex::new(0)];
        if let Some(arr) = nbt.get_int_array("CookingTotalTimes") {
            for (i, &val) in arr.iter().enumerate().take(4) {
                cooking_total_times[i] = Mutex::new(val);
            }
        }

        Self {
            position,
            items,
            cooking_times,
            cooking_total_times,
            dirty: AtomicBool::new(false),
        }
    }

    fn write_nbt<'a>(
        &'a self,
        nbt: &'a mut NbtCompound,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let mut list = Vec::new();
            for (i, item_mutex) in self.items.iter().enumerate() {
                let stack = item_mutex.lock().await;
                if !stack.is_empty() {
                    let mut item_nbt = NbtCompound::new();
                    item_nbt.put_byte("Slot", i as i8);
                    stack.write_item_stack(&mut item_nbt);
                    list.push(NbtTag::Compound(item_nbt));
                }
            }
            nbt.put_list("Items", list);

            let mut times = Vec::new();
            for m in &self.cooking_times {
                times.push(*m.lock().await);
            }
            nbt.put("CookingTimes", NbtTag::IntArray(times));

            let mut total_times = Vec::new();
            for m in &self.cooking_total_times {
                total_times.push(*m.lock().await);
            }
            nbt.put("CookingTotalTimes", NbtTag::IntArray(total_times));
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn tick<'a>(
        &'a self,
        world: &'a Arc<crate::world::World>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let state = world.get_block_state(&self.position);
            let lit = pumpkin_data::block_properties::CampfireLikeProperties::from_state_id(
                state.id,
                world.get_block(&self.position),
            )
            .lit;

            for slot in 0..self.items.len() {
                let input = self.items[slot].lock().await.clone();
                let Some(recipe) = (!input.is_empty())
                    .then(|| {
                        get_cooking_recipe_with_ingredient(
                            input.get_item(),
                            CookingRecipeKind::CampfireCooking,
                        )
                    })
                    .flatten()
                else {
                    *self.cooking_times[slot].lock().await = 0;
                    continue;
                };

                let total = recipe.cookingtime.max(1);
                *self.cooking_total_times[slot].lock().await = total;
                let mut time = self.cooking_times[slot].lock().await;
                if !lit {
                    *time = 0;
                    continue;
                }
                *time += 1;
                if *time < total {
                    self.dirty.store(true, Ordering::Relaxed);
                    continue;
                }
                *time = 0;
                drop(time);

                // Vanilla ejects the cooked result into the world instead of
                // leaving it in the campfire slot.  Replace the input only
                // after the recipe was resolved so component-bearing inputs
                // cannot be lost when a datapack reload removes a recipe.
                let mut item = self.items[slot].lock().await;
                if item.are_items_and_components_equal(&input) {
                    item.clear();
                    if let Some(result_item) =
                        pumpkin_data::item::Item::from_registry_key(recipe.result.id)
                    {
                        world
                            .drop_stack(
                                &self.position,
                                ItemStack::new(recipe.result.count, result_item),
                            )
                            .await;
                    }
                    self.dirty.store(true, Ordering::Relaxed);
                }
            }
        })
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Relaxed);
    }
}

impl CampfireBlockEntity {
    pub const ID: &'static str = "minecraft:campfire";
    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            items: std::array::from_fn(|_| Arc::new(Mutex::new(ItemStack::EMPTY.clone()))),
            cooking_times: [Mutex::new(0), Mutex::new(0), Mutex::new(0), Mutex::new(0)],
            cooking_total_times: [Mutex::new(0), Mutex::new(0), Mutex::new(0), Mutex::new(0)],
            dirty: AtomicBool::new(false),
        }
    }
}
