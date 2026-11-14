//! 实际区块与可编辑区段清单；不从选区或包围范围推断存档中的空洞。
use super::*;

fn chunk_cells(
    clip: &BlockClipboard,
    chunk: [i32; 2],
) -> impl Iterator<Item = ([i64; 3], &State, Option<&str>, Option<i32>)> {
    let width = usize::from(clip.size[0]);
    let depth = usize::from(clip.size[2]);
    let x0 =
        (i64::from(chunk[0]) * 16 - i64::from(clip.source_min[0])).clamp(0, width as i64) as usize;
    let x1 = (i64::from(chunk[0]) * 16 + 16 - i64::from(clip.source_min[0])).clamp(0, width as i64)
        as usize;
    let z0 =
        (i64::from(chunk[1]) * 16 - i64::from(clip.source_min[2])).clamp(0, depth as i64) as usize;
    let z1 = (i64::from(chunk[1]) * 16 + 16 - i64::from(clip.source_min[2])).clamp(0, depth as i64)
        as usize;
    (0..usize::from(clip.size[1]))
        .flat_map(move |y| (z0..z1).flat_map(move |z| (x0..x1).map(move |x| [x, y, z])))
        .filter_map(move |local| {
            let index = (local[1] * depth + local[2]) * width + local[0];
            let state = &clip.blocks[index];
            if clip.mask.as_ref().is_some_and(|mask| !mask[index]) {
                return None;
            }
            let point =
                std::array::from_fn(|axis| i64::from(clip.source_min[axis]) + local[axis] as i64);
            if point[0].div_euclid(16) != i64::from(chunk[0])
                || point[2].div_euclid(16) != i64::from(chunk[1])
            {
                return None;
            }
            let biome = clip.biomes.as_ref().map(|volume| {
                let step = i64::from(volume.resolution);
                let at: [usize; 3] = std::array::from_fn(|axis| {
                    (point[axis].div_euclid(step)
                        - i64::from(clip.source_min[axis]).div_euclid(step))
                        as usize
                });
                volume.cells[(at[1] * usize::from(volume.size[2]) + at[2])
                    * usize::from(volume.size[0])
                    + at[0]]
                    .as_str()
            });
            let numeric = clip
                .numeric_biomes
                .as_ref()
                .map(|volume| volume.cells[index]);
            Some((point, state, biome, numeric))
        })
}
fn structure_chunk_dirty(before: &BlockClipboard, after: &BlockClipboard, chunk: [i32; 2]) -> bool {
    if chunk_cells(before, chunk).ne(chunk_cells(after, chunk)) {
        return true;
    }
    let be = |clip: &BlockClipboard, position: [i32; 3]| {
        (i64::from(position[0]) + i64::from(clip.source_min[0])).div_euclid(16)
            == i64::from(chunk[0])
            && (i64::from(position[2]) + i64::from(clip.source_min[2])).div_euclid(16)
                == i64::from(chunk[1])
    };
    if before
        .block_entities
        .iter()
        .filter(|root| be(before, root.position))
        .map(|root| &root.compound)
        .ne(after
            .block_entities
            .iter()
            .filter(|root| be(after, root.position))
            .map(|root| &root.compound))
    {
        return true;
    }
    let entity = |clip: &BlockClipboard, position: [f64; 3]| {
        ((position[0] + f64::from(clip.source_min[0])) / 16.).floor() == f64::from(chunk[0])
            && ((position[2] + f64::from(clip.source_min[2])) / 16.).floor() == f64::from(chunk[1])
    };
    if before
        .entities
        .iter()
        .filter(|root| entity(before, root.position))
        .map(|root| (&root.compound, root.block_position))
        .ne(after
            .entities
            .iter()
            .filter(|root| entity(after, root.position))
            .map(|root| (&root.compound, root.block_position)))
    {
        return true;
    }
    before
        .scheduled_ticks
        .iter()
        .filter(|tick| be(before, tick.position))
        .ne(after
            .scheduled_ticks
            .iter()
            .filter(|tick| be(after, tick.position)))
}
impl Working {
    pub fn source_revision(
        &mut self,
        world: usize,
        dimension: &str,
    ) -> Result<Arc<()>, WorldError> {
        self.stop()?;
        if let Some(entry) = self.structures.get(&world) {
            if entry.working.content().dimension != dimension {
                return Err(refuse("结构来源维度不一致"));
            }
            return Ok(entry.working.revision());
        }
        Ok(self.entry(world, dimension)?.working.revision())
    }
    fn structure_chunk_cells(
        &self,
        world: usize,
        dimension: &str,
        position: Option<[i32; 2]>,
    ) -> Result<BTreeMap<(i32, i32), BTreeSet<i32>>, WorldError> {
        let entry = &self.structures[&world];
        let content = entry.working.content();
        if content.dimension != dimension {
            return Err(refuse("结构来源维度不一致"));
        }
        let mut chunks = BTreeMap::<_, BTreeSet<_>>::new();
        if let Some(position) = position {
            let mut sections = BTreeSet::new();
            for (index, (point, _, _, _)) in chunk_cells(&content, position).enumerate() {
                if index % 4096 == 0 {
                    self.stop()?;
                }
                sections.insert(point[1].div_euclid(16) as i32);
            }
            if !sections.is_empty() {
                chunks.insert((position[0], position[1]), sections);
            }
            return Ok(chunks);
        }
        if content.mask.is_none() {
            let bounds = entry.working.script_bounds()?;
            let min = bounds.minimum.map(|value| value.div_euclid(16));
            let max = bounds.maximum.map(|value| value.div_euclid(16));
            let count = (i64::from(max[0]) - i64::from(min[0]) + 1)
                * (i64::from(max[2]) - i64::from(min[2]) + 1);
            let bytes = usize::try_from(count)
                .map_err(|_| refuse("结构区块清单超出地址范围"))?
                .checked_mul((max[1] - min[1] + 2) as usize)
                .and_then(|count| count.checked_mul(64))
                .ok_or_else(|| refuse("结构区块清单工作集溢出"))?;
            self.transient_budget()?.check(bytes)?;
            for z in min[2]..=max[2] {
                for x in min[0]..=max[0] {
                    self.stop()?;
                    chunks.insert((x, z), (min[1]..=max[1]).collect());
                }
            }
            return Ok(chunks);
        }
        for index in 0..content.blocks.len() {
            if index % 4096 == 0 {
                self.stop()?;
            }
            if content.mask.as_ref().is_some_and(|mask| !mask[index]) {
                continue;
            }
            let x = index % usize::from(content.size[0]);
            let z = index / usize::from(content.size[0]) % usize::from(content.size[2]);
            let y = index / (usize::from(content.size[0]) * usize::from(content.size[2]));
            let x = content.source_min[0]
                .checked_add(x as i32)
                .ok_or_else(|| refuse("结构区块坐标溢出"))?
                .div_euclid(16);
            let z = content.source_min[2]
                .checked_add(z as i32)
                .ok_or_else(|| refuse("结构区块坐标溢出"))?
                .div_euclid(16);
            if position.is_some_and(|wanted| wanted != [x, z]) {
                continue;
            }
            let y = content.source_min[1]
                .checked_add(y as i32)
                .ok_or_else(|| refuse("结构区段坐标溢出"))?
                .div_euclid(16);
            chunks.entry((x, z)).or_default().insert(y);
        }
        self.transient_budget()?
            .check(chunks.len().saturating_mul(128))?;
        Ok(chunks)
    }
    pub(super) fn script_chunk_positions(
        &mut self,
        world: usize,
        dimension: &str,
    ) -> Result<Value, WorldError> {
        self.stop()?;
        let positions = if self.structures.contains_key(&world) {
            self.structure_chunk_cells(world, dimension, None)?
                .into_keys()
                .collect::<Vec<_>>()
        } else {
            let budget = self.transient_budget()?;
            let token = Arc::clone(&self.cancelled);
            self.entry(world, dimension)?
                .working
                .terrain_chunks_cancellable(budget, || token.load(Ordering::Acquire))?
                .into_iter()
                .collect()
        };
        self.observed.insert((world, dimension.to_owned()));
        serde_json::to_value(positions).map_err(|error| refuse(error.to_string()))
    }
    pub(super) fn script_chunk_info(
        &mut self,
        world: usize,
        dimension: &str,
        position: [i32; 2],
    ) -> Result<Value, WorldError> {
        self.stop()?;
        let [cx, cz] = position;
        let min_x = cx.checked_mul(16).ok_or_else(|| refuse("区块坐标溢出"))?;
        let min_z = cz.checked_mul(16).ok_or_else(|| refuse("区块坐标溢出"))?;
        let max_x = min_x
            .checked_add(15)
            .ok_or_else(|| refuse("区块坐标溢出"))?;
        let max_z = min_z
            .checked_add(15)
            .ok_or_else(|| refuse("区块坐标溢出"))?;
        let value = if let Some(entry) = self.structures.get(&world) {
            let mut chunks = self.structure_chunk_cells(world, dimension, Some(position))?;
            let Some(sections) = chunks.remove(&(cx, cz)) else {
                return Ok(Value::Null);
            };
            let bounds = entry.working.script_bounds()?;
            let bounds = Bounds {
                minimum: [
                    min_x.max(bounds.minimum[0]),
                    bounds.minimum[1],
                    min_z.max(bounds.minimum[2]),
                ],
                maximum: [
                    max_x.min(bounds.maximum[0]),
                    bounds.maximum[1],
                    max_z.min(bounds.maximum[2]),
                ],
            };
            let before = entry.baseline.content();
            let after = entry.working.content();
            let dirty = structure_chunk_dirty(&before, &after, position);
            json!({"bounds":bounds,"sections":sections,"dirty":dirty,"numeric":after.numeric})
        } else {
            let entry = self.entry(world, dimension)?;
            let (chunk, _) = entry.working.visible_chunk_base(cx, cz)?;
            let Some(chunk) = chunk else {
                return Ok(Value::Null);
            };
            let mut sections = chunk
                .sections
                .iter()
                .map(|section| i32::from(section.y))
                .collect::<BTreeSet<_>>();
            sections.extend(
                entry
                    .working
                    .display_changes_in_chunk(cx, cz)
                    .map(|((_, y, _), _)| y.div_euclid(16)),
            );
            if let Some(created) = entry.working.creation.get(&(cx, cz)) {
                sections.extend(
                    created
                        .section_proofs()
                        .iter()
                        .map(|proof| i32::from(proof.section_y)),
                );
            }
            let (min_y, max_y) = SelectionRegion::height(&entry.working.world, dimension)?;
            let numeric = matches!(chunk.layout, Layout::McRegion128 | Layout::NumericAnvil112);
            json!({"bounds":Bounds {minimum:[min_x,min_y,min_z],maximum:[max_x,max_y,max_z]},
                "sections":sections,"dirty":entry.working.dirty_chunks.contains(&(cx,cz)),"numeric":numeric})
        };
        self.observed.insert((world, dimension.to_owned()));
        self.stop()?;
        Ok(value)
    }
}
