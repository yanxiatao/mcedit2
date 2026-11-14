use mcedit2_world::{
    clipboard::SourceFileProof,
    editing::{parse_state, Bounds, EditWorkBudget},
    structure_document::{Document, FillOptions, SourceVersion},
    structure_exchange::Format,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mcedit-budget-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn structure_open_edit_and_save_use_the_selected_budget_without_partial_writes() {
    let directory = Directory::new();
    let path = directory.0.join("copy.schematic");
    // Minimal, fully representable legacy structure; imported testbed files
    // contain unrelated top-level fields that an editable document must retain.
    use mcedit2_world::nbt_document::{Field, List, Value};
    let fields = [
        ("Width", Value::Short(1)),
        ("Height", Value::Short(1)),
        ("Length", Value::Short(1)),
        ("Materials", Value::String("Alpha".into())),
        ("Blocks", Value::ByteArray(vec![1])),
        ("Data", Value::ByteArray(vec![0])),
        (
            "Entities",
            Value::List(List {
                element_type: 10,
                items: vec![],
            }),
        ),
        (
            "TileEntities",
            Value::List(List {
                element_type: 10,
                items: vec![],
            }),
        ),
    ]
    .into_iter()
    .map(|(name, value)| Field {
        name: name.into(),
        value,
    })
    .collect();
    let mut raw = vec![10, 0, 0];
    raw.extend(Value::Compound(fields).payload().unwrap());
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&raw).unwrap();
    fs::write(&path, gzip.finish().unwrap()).unwrap();
    let version = Some(SourceVersion {
        data_version: None,
        release: None,
    });
    let tiny = EditWorkBudget::from_bytes(1).unwrap();
    let budget = EditWorkBudget::default();
    let original = fs::read(&path).unwrap();
    assert!(Document::open(&path, Format::Legacy, version.clone(), tiny).is_err());
    let mut document = Document::open(&path, Format::Legacy, version.clone(), budget).unwrap();
    let id = if document.content().blocks[0].name == "numeric:1" {
        0
    } else {
        1
    };
    let replacement = parse_state(&format!("numeric:{id}[data=0]")).unwrap();
    document
        .fill(
            Bounds::new((0, 0, 0), (0, 0, 0)).unwrap().into(),
            &replacement,
            FillOptions {
                remove_block_entities: true,
                ..Default::default()
            },
            budget,
            || false,
        )
        .unwrap();
    assert!(document.prepare_save(tiny).is_err());
    assert!(document.dirty());
    assert_eq!(fs::read(&path).unwrap(), original);
    document.save(budget).unwrap();
    assert!(!document.dirty());
    let reopened = Document::open(&path, Format::Legacy, version, budget).unwrap();
    assert_eq!(reopened.content().blocks[0], replacement);
}

#[test]
fn source_identity_streams_large_files_and_detects_later_changes() {
    let directory = Directory::new();
    let path = directory.0.join("large.source");
    let length = 65 * 1024 * 1024;
    let mut file = fs::File::create(&path).unwrap();
    file.set_len(length).unwrap();
    let mut hash = Sha256::new();
    for _ in 0..length / 65536 {
        hash.update([0u8; 65536]);
    }
    let proof = SourceFileProof {
        path: path.canonicalize().unwrap(),
        sha256: format!("{:x}", hash.finalize()),
    };
    proof.verify().unwrap();
    file.seek(SeekFrom::Start(length - 1)).unwrap();
    file.write_all(&[1]).unwrap();
    file.sync_all().unwrap();
    assert!(proof.verify().is_err());
}
