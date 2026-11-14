//! 静态命令导航的参数边界；引号、转义及嵌套选择器 / SNBT 保留原文。
pub(super) fn split(text: &str, comma: bool) -> Result<Vec<&str>, String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut brackets = Vec::new();
    for (index, ch) in text.char_indices() {
        if let Some(mark) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == mark {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '[' | '{' => brackets.push(ch),
            ']' | '}' => {
                let opening = if ch == ']' { '[' } else { '{' };
                if brackets.pop() != Some(opening) {
                    return Err(format!("命令括号不匹配，位置 {index}"));
                }
            }
            _ if brackets.is_empty() && if comma { ch == ',' } else { ch.is_whitespace() } => {
                let part = text[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                } else if comma {
                    return Err("选择器含空参数".into());
                }
                start = index + ch.len_utf8();
            }
            _ => (),
        }
    }
    if quote.is_some() || !brackets.is_empty() {
        return Err("命令引号或括号未闭合".into());
    }
    let last = text[start..].trim();
    if !last.is_empty() {
        parts.push(last);
    } else if comma && !text.is_empty() {
        return Err("选择器含空参数".into());
    }
    Ok(parts)
}
