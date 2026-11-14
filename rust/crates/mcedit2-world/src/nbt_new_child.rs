//! 用户明确新增节点时的标准初值、原版字段命名与实际列表类型合同。
use super::{
    children::length, parse_path, parse_root, refuse, resolve, Document, List, Operation, Result,
    Tag, Value,
};
use std::collections::BTreeSet;

impl Value {
    /// 创建用户所选 NBT 类型的标准空 / 零初值，不用于缺失或损坏节点的替代。
    pub fn new_tag(kind: u8) -> Result<Self> {
        Ok(match kind {
            1 => Self::Byte(0),
            2 => Self::Short(0),
            3 => Self::Int(0),
            4 => Self::Long(0),
            5 => Self::Float(0.),
            6 => Self::Double(0.),
            7 => Self::ByteArray(Vec::new()),
            8 => Self::String(String::new()),
            9 => Self::List(List {
                element_type: 0,
                items: Vec::new(),
            }),
            10 => Self::Compound(Vec::new()),
            11 => Self::IntArray(Vec::new()),
            12 => Self::LongArray(Vec::new()),
            _ => return Err(refuse("无效 NBT 新节点类型")),
        })
    }
}
fn child_type(node: &Tag<'_>) -> Result<Option<u8>> {
    Ok(match node.kind {
        10 => None,
        9 => (node.raw[0] != 0).then_some(node.raw[0]),
        7 => Some(1),
        11 => Some(3),
        12 => Some(4),
        _ => return Err(refuse("新增节点的目标必须为 Compound、List 或数组")),
    })
}
impl Document {
    /// 固定类型列表 / 数组返回真实元素类型；Compound 和未声明类型的空列表须明确选择。
    pub fn child_type(&self, path: &str) -> Result<Option<u8>> {
        let root = parse_root(&self.bytes).map_err(|error| refuse(error.to_string()))?;
        child_type(resolve(&root, &parse_path(path)?)?)
    }
    /// 生成一次原生插入操作；读取实际字段集合与元素类型，不修改文档或重排旧载荷。
    pub fn new_child(&self, path: &str, kind: u8) -> Result<Operation> {
        let root = parse_root(&self.bytes).map_err(|error| refuse(error.to_string()))?;
        let node = resolve(&root, &parse_path(path)?)?;
        let fixed = child_type(node)?;
        if fixed.is_some_and(|fixed| fixed != kind) {
            return Err(refuse("新增节点类型与实际列表 / 数组元素类型不符"));
        }
        let value = Value::new_tag(kind)?;
        if node.kind == 10 {
            let names: BTreeSet<_> = node.fields.iter().map(|(name, _)| name.as_str()).collect();
            let mut name = "Unnamed".to_owned();
            let mut number = 0;
            while names.contains(name.as_str()) {
                number += 1;
                name = format!("Unnamed {number}");
            }
            Ok(Operation::InsertField {
                path: path.into(),
                name,
                value,
            })
        } else {
            Ok(Operation::InsertItem {
                path: path.into(),
                index: length(node)?,
                value,
            })
        }
    }
}
