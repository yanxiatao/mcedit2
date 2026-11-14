//! GPU 保留预备列，实际呈现和拾取只采用当前范围；范围切换不重新上传已常驻几何。
use super::*;

struct Scoped {
    source: Arc<crate::spectator::StreamMesh>,
    columns: crate::spectator_stream::Columns,
    topology_matches: bool,
    mesh: Arc<crate::spectator::StreamMesh>,
}
#[derive(Default)]
pub(super) struct Cache(Option<Scoped>);
impl Cache {
    pub(super) fn clear(&mut self) {
        self.0 = None;
    }
    pub(super) fn prepare(
        &mut self,
        source: &Arc<crate::spectator::StreamMesh>,
        columns: Option<&crate::spectator_stream::Columns>,
        topology_matches: bool,
    ) -> Arc<crate::spectator::StreamMesh> {
        let Some(columns) = columns else {
            self.clear();
            return Arc::clone(source);
        };
        if let Some(previous) = &self.0 {
            if Arc::ptr_eq(&previous.source, source)
                && previous.columns == *columns
                && previous.topology_matches == topology_matches
            {
                return Arc::clone(&previous.mesh);
            }
        }
        let allowed = |part: &&crate::spectator::MeshChunk| {
            part.key.2 != 1 || columns.contains(&(part.key.0, part.key.1))
        };
        let removes = source.chunks.iter().any(|part| !allowed(&part));
        let mesh = if removes || (!topology_matches && source.visibility.is_some()) {
            let mut mesh = (**source).clone();
            if removes {
                mesh.chunks = source
                    .chunks
                    .iter()
                    .filter(allowed)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into();
            }
            if !topology_matches {
                mesh.visibility = None;
            }
            mesh.terrain = Some(Arc::clone(source.terrain.as_ref().unwrap_or(source)));
            Arc::new(mesh)
        } else {
            Arc::clone(source)
        };
        self.0 = Some(Scoped {
            source: Arc::clone(source),
            columns: Arc::clone(columns),
            topology_matches,
            mesh: Arc::clone(&mesh),
        });
        mesh
    }
}
