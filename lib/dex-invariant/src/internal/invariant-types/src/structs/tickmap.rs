use std::{convert::TryInto, fmt::Debug};

use crate::{size, MAX_VIRTUAL_CROSS};
use anchor_lang::prelude::*;

use crate::utils::{TrackableError, TrackableResult};
use crate::{err, function, location, trace};

pub const TICK_LIMIT: i32 = 44_364; // If you change it update length of array as well!
pub const TICK_SEARCH_RANGE: i32 = 256;
pub const MAX_TICK: i32 = 221_818; // log(1.0001, sqrt(2^64-1))
pub const TICK_CROSSES_PER_IX: usize = 10;
pub const TICKS_BACK_COUNT: usize = 1;
pub const TICKMAP_SIZE: i32 = 2 * TICK_LIMIT - 1;

const TICKMAP_RANGE: usize = (TICK_CROSSES_PER_IX + TICKS_BACK_COUNT + MAX_VIRTUAL_CROSS as usize)
    * TICK_SEARCH_RANGE as usize;
const TICKMAP_SLICE_SIZE: usize = TICKMAP_RANGE / 8 + 2;

pub fn tick_to_position(tick: i32, tick_spacing: u16) -> (usize, u8) {
    assert_eq!(
        (tick % tick_spacing as i32),
        0,
        "tick not divisible by spacing"
    );

    let bitmap_index = tick
        .checked_div(tick_spacing.try_into().unwrap())
        .unwrap()
        .checked_add(TICK_LIMIT)
        .unwrap();

    let byte: usize = (bitmap_index.checked_div(8).unwrap()).try_into().unwrap();
    let bit: u8 = (bitmap_index % 8).abs().try_into().unwrap();

    (byte, bit)
}

// tick_spacing - spacing already scaled by tick_spacing
pub fn get_search_limit(tick: i32, tick_spacing: u16, up: bool) -> i32 {
    let index = tick / tick_spacing as i32;

    // limit unsclaed
    let limit = if up {
        // ticks are limited by amount of space in the bitmap...
        let array_limit = TICK_LIMIT.checked_sub(1).unwrap();
        // ...search range is limited to 256 at the time ...
        let range_limit = index.checked_add(TICK_SEARCH_RANGE).unwrap();
        // ...also ticks for prices over 2^64 aren't needed
        let price_limit = MAX_TICK.checked_div(tick_spacing as i32).unwrap();

        array_limit.min(range_limit).min(price_limit)
    } else {
        let array_limit = (-TICK_LIMIT).checked_add(1).unwrap();
        let range_limit = index.checked_sub(TICK_SEARCH_RANGE).unwrap();
        let price_limit = -MAX_TICK.checked_div(tick_spacing as i32).unwrap();

        array_limit.max(range_limit).max(price_limit)
    };

    // scaled by tick_spacing
    limit.checked_mul(tick_spacing as i32).unwrap()
}

#[account(zero_copy(unsafe))]
#[repr(packed)]
#[derive(AnchorDeserialize)]
pub struct Tickmap {
    pub bitmap: [u8; 11091], // Tick limit / 4
}

impl Default for Tickmap {
    fn default() -> Self {
        Tickmap { bitmap: [0; 11091] }
    }
}
impl Debug for Tickmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?}",
            self.bitmap.iter().fold(0, |acc, v| acc + v.count_ones())
        )
    }
}
size!(Tickmap);

impl Tickmap {
    pub fn get(&self, tick: i32, tick_spacing: u16) -> bool {
        let (byte, bit) = tick_to_position(tick, tick_spacing);
        let value = (self.bitmap[byte] >> bit) % 2;

        (value) == 1
    }

    pub fn flip(&mut self, value: bool, tick: i32, tick_spacing: u16) {
        assert!(
            self.get(tick, tick_spacing) != value,
            "tick initialize tick again"
        );

        let (byte, bit) = tick_to_position(tick, tick_spacing);

        self.bitmap[byte] ^= 1 << bit;
    }
}
pub struct TickmapSlice {
    pub data: [u8; TICKMAP_SLICE_SIZE],
    pub offset: i32,
}
impl Default for TickmapSlice {
    fn default() -> Self {
        Self {
            data: [0u8; TICKMAP_SLICE_SIZE],
            offset: 0,
        }
    }
}

impl TickmapSlice {
    pub fn calculate_search_range_offset(init_tick: i32, spacing: u16, up: bool) -> i32 {
        let search_limit = get_search_limit(init_tick, spacing, up);
        let position = tick_to_position(search_limit, spacing).0 as i32;

        if up {
            position - TICKMAP_SLICE_SIZE as i32 + 1
        } else {
            position
        }
    }

    pub fn from_slice(
        tickmap_data: &[u8],
        current_tick_index: i32,
        tick_spacing: u16,
        x_to_y: bool,
    ) -> TrackableResult<Self> {
        let offset = if x_to_y {
            TICK_SEARCH_RANGE - TICKMAP_SLICE_SIZE as i32 * 8 - 8
        } else {
            -TICK_SEARCH_RANGE + 8
        };

        let start_index = ((current_tick_index / tick_spacing as i32 + TICK_LIMIT + offset) / 8)
            .max(0)
            .min((TICKMAP_SIZE + 1) / 8 - TICKMAP_SLICE_SIZE as i32)
            .try_into()
            .map_err(|_| err!("Failed to set start_index"))?;
        let end_index = (start_index as i32 + TICKMAP_SLICE_SIZE as i32)
            .min(tickmap_data.len() as i32)
            .try_into()
            .map_err(|_| err!("Failed to set end_index"))?;

        let mut data = [0u8; TICKMAP_SLICE_SIZE];
        data[..end_index - start_index].copy_from_slice(&tickmap_data[start_index..end_index]);

        Ok(TickmapSlice {
            data,
            offset: start_index as i32,
        })
    }

    pub fn get(&self, index: usize) -> Option<&u8> {
        let index = index.checked_sub(self.offset as usize)?;
        self.data.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut u8> {
        let index = index.checked_sub(self.offset as usize)?;
        self.data.get_mut(index)
    }
}

impl std::ops::Index<usize> for TickmapSlice {
    type Output = u8;
    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).unwrap()
    }
}

impl std::ops::IndexMut<usize> for TickmapSlice {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index).unwrap()
    }
}

#[derive(Default)]
pub struct TickmapView {
    pub bitmap: TickmapSlice,
}

impl std::fmt::Debug for TickmapView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self
            .bitmap
            .data
            .iter()
            .fold(0, |acc, v| acc + v.count_ones());
        write!(f, "{:?}", count)
    }
}

impl TickmapView {
    pub fn next_initialized(&self, tick: i32, tick_spacing: u16) -> Option<i32> {
        let limit = get_search_limit(tick, tick_spacing, true);

        // add 1 to not check current tick
        let (mut byte, mut bit) =
            tick_to_position(tick.checked_add(tick_spacing as i32).unwrap(), tick_spacing);
        let (limiting_byte, limiting_bit) = tick_to_position(limit, tick_spacing);

        while byte < limiting_byte || (byte == limiting_byte && bit <= limiting_bit) {
            // ignore some bits on first loop
            let (limiting_byte, limiting_bit) = tick_to_position(limit, tick_spacing);
            let mut shifted = self.bitmap[byte] >> bit;

            // go through all bits in byte until it is zero
            if shifted != 0 {
                while shifted.checked_rem(2).unwrap() == 0 {
                    shifted >>= 1;
                    bit = bit.checked_add(1).unwrap();
                }

                return if byte < limiting_byte || (byte == limiting_byte && bit <= limiting_bit) {
                    let index: i32 = byte
                        .checked_mul(8)
                        .unwrap()
                        .checked_add(bit.into())
                        .unwrap()
                        .try_into()
                        .unwrap();
                    Some(
                        index
                            .checked_sub(TICK_LIMIT)
                            .unwrap()
                            .checked_mul(tick_spacing.try_into().unwrap())
                            .unwrap(),
                    )
                } else {
                    None
                };
            }

            // go to the text byte
            if let Some(value) = byte.checked_add(1) {
                byte = value;
            } else {
                return None;
            }
            bit = 0;
        }

        None
    }

    // tick_spacing - spacing already scaled by tick_spacing
    pub fn prev_initialized(&self, tick: i32, tick_spacing: u16) -> Option<i32> {
        // don't subtract 1 to check the current tick
        let limit = get_search_limit(tick, tick_spacing, false); // limit scaled by tick_spacing
        let (mut byte, mut bit) = tick_to_position(tick as i32, tick_spacing);
        let (limiting_byte, limiting_bit) = tick_to_position(limit, tick_spacing);

        while byte > limiting_byte || (byte == limiting_byte && bit >= limiting_bit) {
            // always safe due to limitated domain of bit variable
            let mut mask = 1u16.checked_shl(bit.try_into().unwrap()).unwrap(); // left = MSB direction (increase value)
            let value = self.bitmap[byte] as u16;

            // enter if some of previous bits are initialized in current byte
            if value.checked_rem(mask.checked_shl(1).unwrap()).unwrap() > 0 {
                // skip uninitalized ticks
                while value & mask == 0 {
                    mask >>= 1;
                    bit = bit.checked_sub(1).unwrap();
                }

                // return first initalized tick if limiit is not exceeded, otherswise return None
                return if byte > limiting_byte || (byte == limiting_byte && bit >= limiting_bit) {
                    // no possibility to overflow
                    let index: i32 = byte
                        .checked_mul(8)
                        .unwrap()
                        .checked_add(bit.into())
                        .unwrap()
                        .try_into()
                        .unwrap();

                    Some(
                        index
                            .checked_sub(TICK_LIMIT)
                            .unwrap()
                            .checked_mul(tick_spacing.try_into().unwrap())
                            .unwrap(),
                    )
                } else {
                    None
                };
            }

            // go to the next byte
            if let Some(value) = byte.checked_sub(1) {
                byte = value;
            } else {
                return None;
            }
            bit = 7;
        }

        None
    }

    pub fn get(&self, tick: i32, tick_spacing: u16) -> bool {
        let (byte, bit) = tick_to_position(tick, tick_spacing);
        let value = (self.bitmap[byte] >> bit) % 2;

        (value) == 1
    }

    pub fn flip(&mut self, value: bool, tick: i32, tick_spacing: u16) {
        assert!(
            self.get(tick, tick_spacing) != value,
            "tick initialize tick again"
        );

        let (byte, bit) = tick_to_position(tick, tick_spacing);

        self.bitmap[byte] ^= 1 << bit;
    }

    pub fn from_slice(
        tickmap_data: &[u8],
        current_tick_index: i32,
        tick_spacing: u16,
        x_to_y: bool,
    ) -> TrackableResult<Self> {
        let bitmap =
            TickmapSlice::from_slice(tickmap_data, current_tick_index, tick_spacing, x_to_y)?;
        Ok(Self { bitmap })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_and_prev_initialized() {
        // initalized edges
        {
            for spacing in 1..=10 {
                println!("spacing = {}", spacing);
                let max_index = match spacing < 5 {
                    true => TICK_LIMIT - spacing,
                    false => (MAX_TICK / spacing) * spacing,
                };
                let min_index = -max_index;
                println!("max_index = {}", max_index);
                println!("min_index = {}", min_index);
                let offset_high =
                    TickmapSlice::calculate_search_range_offset(max_index, spacing as u16, true);
                let offset_low =
                    TickmapSlice::calculate_search_range_offset(min_index, spacing as u16, false);

                let mut map_low = TickmapView {
                    bitmap: TickmapSlice {
                        offset: offset_low,
                        ..Default::default()
                    },
                };
                let mut map_high = TickmapView {
                    bitmap: TickmapSlice {
                        offset: offset_high,
                        ..Default::default()
                    },
                };
                map_low.flip(true, min_index, spacing as u16);
                map_high.flip(true, max_index, spacing as u16);

                let tick_edge_diff = TICK_SEARCH_RANGE / spacing * spacing;

                let prev = map_low.prev_initialized(min_index + tick_edge_diff, spacing as u16);
                let next = map_high.next_initialized(max_index - tick_edge_diff, spacing as u16);

                if prev.is_some() {
                    println!("found prev = {}", prev.unwrap());
                }
                if next.is_some() {
                    println!("found next = {}", next.unwrap());
                }
            }
        }
        // unintalized edges
        for spacing in 1..=1000 {
            let max_index = match spacing < 5 {
                true => TICK_LIMIT - spacing,
                false => (MAX_TICK / spacing) * spacing,
            };
            let min_index = -max_index;

            let tick_edge_diff = TICK_SEARCH_RANGE / spacing * spacing;

            let offset_high =
                TickmapSlice::calculate_search_range_offset(max_index, spacing as u16, true);
            let offset_low =
                TickmapSlice::calculate_search_range_offset(min_index, spacing as u16, false);
            let map_low = TickmapView {
                bitmap: TickmapSlice {
                    offset: offset_low,
                    ..Default::default()
                },
            };
            let map_high = TickmapView {
                bitmap: TickmapSlice {
                    offset: offset_high,
                    ..Default::default()
                },
            };

            let prev = map_low.prev_initialized(min_index + tick_edge_diff, spacing as u16);
            let next = map_high.next_initialized(max_index - tick_edge_diff, spacing as u16);

            if prev.is_some() {
                println!("found prev = {}", prev.unwrap());
            }
            if next.is_some() {
                println!("found next = {}", next.unwrap());
            }
        }
    }
    #[test]
    fn test_slice_edges() {
        let spacing = 1;
        // low_bit == 0
        {
            let mut tickmap = Tickmap::default();
            let low_byte = 0;
            let low_bit = 0;
            let low_tick = low_byte * 8 + low_bit - TICK_LIMIT;

            let high_tick = low_tick + TICKMAP_RANGE as i32;
            let (high_byte, _high_bit) = tick_to_position(high_tick, spacing);

            tickmap.flip(true, low_tick, spacing);
            tickmap.flip(true, high_tick, spacing);
            let tickmap_x_to_y =
                TickmapSlice::from_slice(&tickmap.bitmap, low_tick, spacing, true).unwrap();
            let tickmap_y_to_x =
                TickmapSlice::from_slice(&tickmap.bitmap, low_tick, spacing, false).unwrap();
            assert_eq!(
                tickmap_x_to_y.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_x_to_y.get(high_byte as usize).unwrap(),
                tickmap.bitmap.get(high_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_y_to_x.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_y_to_x.get(high_byte as usize).unwrap(),
                tickmap.bitmap.get(high_byte as usize).unwrap()
            );
        }
        // low_bit == 7
        {
            let mut tickmap = Tickmap::default();
            let low_byte = 0;
            let low_bit = 7;
            let low_tick = low_byte * 8 + low_bit - TICK_LIMIT;

            let high_tick = low_tick + TICKMAP_RANGE as i32;
            let (high_byte, _high_bit) = tick_to_position(high_tick, spacing);

            tickmap.flip(true, low_tick, spacing);
            tickmap.flip(true, high_tick, spacing);
            let tickmap_x_to_y =
                TickmapSlice::from_slice(&tickmap.bitmap, low_tick, spacing, true).unwrap();
            let tickmap_y_to_x =
                TickmapSlice::from_slice(&tickmap.bitmap, low_tick, spacing, false).unwrap();
            assert_eq!(
                tickmap_x_to_y.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_x_to_y.get(high_byte as usize).unwrap(),
                tickmap.bitmap.get(high_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_y_to_x.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_y_to_x.get(high_byte as usize).unwrap(),
                tickmap.bitmap.get(high_byte as usize).unwrap()
            );
        }
        // high_bit = 7
        {
            let mut tickmap = Tickmap::default();
            let high_byte = tickmap.bitmap.len() as i32 - 1;
            let high_bit = 7;
            let high_tick = high_byte * 8 + high_bit - TICK_LIMIT;

            let low_tick = high_tick - TICKMAP_RANGE as i32;
            let (low_byte, _low_bit) = tick_to_position(low_tick, spacing);

            tickmap.flip(true, low_tick, spacing);
            tickmap.flip(true, high_tick, spacing);
            let tickmap_x_to_y =
                TickmapSlice::from_slice(&tickmap.bitmap, high_tick, spacing, true).unwrap();
            let tickmap_y_to_x =
                TickmapSlice::from_slice(&tickmap.bitmap, high_tick, spacing, false).unwrap();
            assert_eq!(
                tickmap_x_to_y.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_x_to_y.get(high_byte as usize).unwrap(),
                tickmap.bitmap.get(high_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_y_to_x.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_y_to_x.get(high_byte as usize).unwrap(),
                tickmap.bitmap.get(high_byte as usize).unwrap()
            );
        }
        // high_bit = 0
        {
            let mut tickmap = Tickmap::default();
            let high_byte = tickmap.bitmap.len() as i32 - 1;
            let high_bit = 0;
            let high_tick = high_byte * 8 + high_bit - TICK_LIMIT;

            let low_tick = high_tick - TICKMAP_RANGE as i32;
            let (low_byte, _low_bit) = tick_to_position(low_tick, spacing);

            tickmap.flip(true, low_tick, spacing);
            tickmap.flip(true, high_tick, spacing);
            let tickmap_x_to_y =
                TickmapSlice::from_slice(&tickmap.bitmap, high_tick, spacing, true).unwrap();
            let tickmap_y_to_x =
                TickmapSlice::from_slice(&tickmap.bitmap, high_tick, spacing, false).unwrap();
            assert_eq!(
                tickmap_x_to_y.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_x_to_y.get(high_byte as usize).unwrap(),
                tickmap.bitmap.get(high_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_y_to_x.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_y_to_x.get(high_byte as usize).unwrap(),
                tickmap.bitmap.get(high_byte as usize).unwrap()
            );
        }
    }

    #[test]
    fn test_ticks_back() {
        let spacing = 1;
        let byte_offset = 10;
        let range_offset_with_tick_back =
            TICKMAP_SLICE_SIZE as i32 * 8 - TICKS_BACK_COUNT as i32 * TICK_SEARCH_RANGE;
        // low_bit == 0
        {
            let mut tickmap = Tickmap::default();
            let low_byte = byte_offset;
            let low_bit = 0;
            let low_tick = low_byte * 8 + low_bit - TICK_LIMIT;

            let high_tick = low_tick + TICKMAP_RANGE as i32;
            let (high_byte, _high_bit) = tick_to_position(high_tick, spacing);

            tickmap.flip(true, low_tick, spacing);
            tickmap.flip(true, high_tick, spacing);
            let tickmap_x_to_y = TickmapSlice::from_slice(
                &tickmap.bitmap,
                low_tick + range_offset_with_tick_back,
                spacing,
                true,
            )
            .unwrap();
            assert_eq!(
                tickmap_x_to_y.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_x_to_y.get(high_byte as usize).unwrap(),
                tickmap.bitmap.get(high_byte as usize).unwrap()
            );
        }
        // low_bit == 7
        {
            let mut tickmap = Tickmap::default();
            let low_byte = byte_offset;
            let low_bit = 7;
            let low_tick = low_byte * 8 + low_bit - TICK_LIMIT;

            let high_tick = low_tick + TICKMAP_RANGE as i32;
            let (high_byte, _high_bit) = tick_to_position(high_tick, spacing);

            tickmap.flip(true, low_tick, spacing);
            tickmap.flip(true, high_tick, spacing);
            let tickmap_x_to_y = TickmapSlice::from_slice(
                &tickmap.bitmap,
                low_tick + range_offset_with_tick_back,
                spacing,
                true,
            )
            .unwrap();
            assert_eq!(
                tickmap_x_to_y.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap.bitmap.get(high_byte as usize).unwrap(),
                tickmap_x_to_y.get(high_byte as usize).unwrap()
            );
        }
        // high_bit = 7
        {
            let mut tickmap = Tickmap::default();
            let high_byte = tickmap.bitmap.len() as i32 - 1 - byte_offset;
            let high_bit = 7;
            let high_tick = high_byte * 8 + high_bit - TICK_LIMIT;

            let low_tick = high_tick - TICKMAP_RANGE as i32;
            let (low_byte, _low_bit) = tick_to_position(low_tick, spacing);

            tickmap.flip(true, low_tick, spacing);
            tickmap.flip(true, high_tick, spacing);
            let tickmap_y_to_x = TickmapSlice::from_slice(
                &tickmap.bitmap,
                high_tick - range_offset_with_tick_back,
                spacing,
                false,
            )
            .unwrap();
            assert_eq!(
                tickmap_y_to_x.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_y_to_x.get(high_byte as usize).unwrap(),
                tickmap.bitmap.get(high_byte as usize).unwrap()
            );
        }
        // high_bit = 0
        {
            let mut tickmap = Tickmap::default();
            let high_byte = tickmap.bitmap.len() as i32 - 1 - byte_offset;
            let high_bit = 0;
            let high_tick = high_byte * 8 + high_bit - TICK_LIMIT;

            let low_tick = high_tick - TICKMAP_RANGE as i32;
            let (low_byte, _low_bit) = tick_to_position(low_tick, spacing);

            tickmap.flip(true, low_tick, spacing);
            tickmap.flip(true, high_tick, spacing);
            let tickmap_y_to_x = TickmapSlice::from_slice(
                &tickmap.bitmap,
                high_tick - range_offset_with_tick_back,
                spacing,
                false,
            )
            .unwrap();
            assert_eq!(
                tickmap_y_to_x.get(low_byte as usize).unwrap(),
                tickmap.bitmap.get(low_byte as usize).unwrap()
            );
            assert_eq!(
                tickmap_y_to_x.get(high_byte as usize).unwrap(),
                tickmap.bitmap.get(high_byte as usize).unwrap()
            );
        }
    }
}
