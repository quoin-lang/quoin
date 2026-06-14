use crate::instruction::{Instruction, Constant, StaticBlock};

struct Builder {
    name: Option<String>,
    is_nested_block: bool,
    param_names: Vec<String>,
    bytecode: Vec<Instruction>,
}

pub fn assemble(source: &str) -> Result<StaticBlock, String> {
    let mut builders = vec![Builder {
        name: Some("main".to_string()),
        is_nested_block: false,
        param_names: Vec::new(),
        bytecode: Vec::new(),
    }];

    for (line_num, raw_line) in source.lines().enumerate() {
        let line_num = line_num + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let tokens = tokenize(trimmed)?;
        if tokens.is_empty() {
            continue;
        }

        let first = &tokens[0];

        if first == "block" || first == "nested_block" {
            if tokens.len() < 2 {
                return Err(format!("Line {}: block requires a name (use '_' for anonymous)", line_num));
            }
            let raw_name = tokens[1].clone();
            let name = if raw_name == "_" { None } else { Some(raw_name) };
            let params = tokens[2..].to_vec();
            builders.push(Builder {
                name,
                is_nested_block: first == "nested_block",
                param_names: params,
                bytecode: Vec::new(),
            });
        } else if first == "end" {
            if builders.len() <= 1 {
                return Err(format!("Line {}: unmatched 'end'", line_num));
            }
            let finished_builder = builders.pop().unwrap();
            let static_block = StaticBlock {
                name: finished_builder.name,
                is_nested_block: finished_builder.is_nested_block,
                param_names: finished_builder.param_names,
                bytecode: finished_builder.bytecode,
            };
            // Push the block as a constant onto the parent builder's bytecode
            builders
                .last_mut()
                .unwrap()
                .bytecode
                .push(Instruction::Push(Constant::Block(static_block)));
        } else {
            let inst = parse_instruction(&tokens, trimmed, line_num)?;
            builders.last_mut().unwrap().bytecode.push(inst);
        }
    }

    if builders.len() != 1 {
        return Err("Unclosed block at end of file".to_string());
    }

    let main_builder = builders.pop().unwrap();
    Ok(StaticBlock {
        name: main_builder.name,
        is_nested_block: main_builder.is_nested_block,
        param_names: main_builder.param_names,
        bytecode: main_builder.bytecode,
    })
}

fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next(); // consume opening quote
            let mut s = String::new();
            let mut closed = false;
            while let Some(nc) = chars.next() {
                if nc == '"' {
                    closed = true;
                    break;
                } else if nc == '\\' {
                    if let Some(escaped) = chars.next() {
                        match escaped {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            '\\' => s.push('\\'),
                            '"' => s.push('"'),
                            _ => s.push(escaped),
                        }
                    } else {
                        return Err("Trailing backslash in string".to_string());
                    }
                } else {
                    s.push(nc);
                }
            }
            if !closed {
                return Err("Unclosed string literal".to_string());
            }
            tokens.push(s);
        } else if c == '#' {
            break; // comment till end of line
        } else {
            let mut s = String::new();
            while let Some(&nc) = chars.peek() {
                if nc.is_whitespace() || nc == '"' || nc == '#' {
                    break;
                }
                s.push(nc);
                chars.next();
            }
            tokens.push(s);
        }
    }

    Ok(tokens)
}

fn parse_instruction(tokens: &[String], original_line: &str, line_num: usize) -> Result<Instruction, String> {
    let op = &tokens[0];
    match op.as_str() {
        "load_local" => {
            let name = tokens.get(1).ok_or_else(|| format!("Line {}: load_local requires a variable name", line_num))?;
            Ok(Instruction::LoadLocal(name.clone()))
        }
        "define_local" => {
            let name = tokens.get(1).ok_or_else(|| format!("Line {}: define_local requires a variable name", line_num))?;
            Ok(Instruction::DefineLocal(name.clone()))
        }
        "store_local" => {
            let name = tokens.get(1).ok_or_else(|| format!("Line {}: store_local requires a variable name", line_num))?;
            Ok(Instruction::StoreLocal(name.clone()))
        }
        "load_global" => {
            let name = tokens.get(1).ok_or_else(|| format!("Line {}: load_global requires a name", line_num))?;
            Ok(Instruction::LoadGlobal(name.clone()))
        }
        "store_global" => {
            let name = tokens.get(1).ok_or_else(|| format!("Line {}: store_global requires a name", line_num))?;
            Ok(Instruction::StoreGlobal(name.clone()))
        }
        "push" => {
            let type_str = tokens.get(1).ok_or_else(|| format!("Line {}: push requires a type (nil, bool, int, float, string)", line_num))?;
            let const_val = match type_str.as_str() {
                "nil" => Constant::Nil,
                "bool" => {
                    let b_str = tokens.get(2).ok_or_else(|| format!("Line {}: push bool requires a value (true/false)", line_num))?;
                    let b = b_str.parse::<bool>().map_err(|_| format!("Line {}: invalid bool value: {}", line_num, b_str))?;
                    Constant::Bool(b)
                }
                "int" => {
                    let i_str = tokens.get(2).ok_or_else(|| format!("Line {}: push int requires an integer value", line_num))?;
                    let i = i_str.parse::<i64>().map_err(|_| format!("Line {}: invalid integer: {}", line_num, i_str))?;
                    Constant::Int(i)
                }
                "float" => {
                    let f_str = tokens.get(2).ok_or_else(|| format!("Line {}: push float requires a float value", line_num))?;
                    let f = f_str.parse::<f64>().map_err(|_| format!("Line {}: invalid float: {}", line_num, f_str))?;
                    Constant::Float(f)
                }
                "string" => {
                    let s = tokens.get(2).ok_or_else(|| format!("Line {}: push string requires a string literal", line_num))?;
                    Constant::String(s.clone())
                }
                _ => return Err(format!("Line {}: unknown push type '{}'", line_num, type_str)),
            };
            Ok(Instruction::Push(const_val))
        }
        "pop" => Ok(Instruction::Pop),
        "dup" => Ok(Instruction::Dup),
        "call" => {
            let n_str = tokens.get(1).ok_or_else(|| format!("Line {}: call requires arg count", line_num))?;
            let n = n_str.parse::<usize>().map_err(|_| format!("Line {}: invalid call count: {}", line_num, n_str))?;
            Ok(Instruction::Call(n))
        }
        "return" => Ok(Instruction::Return),
        "yeet" => Ok(Instruction::Yeet),
        "block_return" => Ok(Instruction::BlockReturn),
        "method_return" => Ok(Instruction::MethodReturn),
        "jump" => {
            let offset_str = tokens.get(1).ok_or_else(|| format!("Line {}: jump requires offset", line_num))?;
            let offset = offset_str.parse::<isize>().map_err(|_| format!("Line {}: invalid jump offset: {}", line_num, offset_str))?;
            Ok(Instruction::Jump(offset))
        }
        "if_jump" => {
            let offset_str = tokens.get(1).ok_or_else(|| format!("Line {}: if_jump requires offset", line_num))?;
            let offset = offset_str.parse::<isize>().map_err(|_| format!("Line {}: invalid jump offset: {}", line_num, offset_str))?;
            Ok(Instruction::IfJump(offset))
        }
        "else_jump" => {
            let offset_str = tokens.get(1).ok_or_else(|| format!("Line {}: else_jump requires offset", line_num))?;
            let offset = offset_str.parse::<isize>().map_err(|_| format!("Line {}: invalid jump offset: {}", line_num, offset_str))?;
            Ok(Instruction::ElseJump(offset))
        }
        "new_list" => {
            let n_str = tokens.get(1).ok_or_else(|| format!("Line {}: new_list requires element count", line_num))?;
            let n = n_str.parse::<usize>().map_err(|_| format!("Line {}: invalid list count: {}", line_num, n_str))?;
            Ok(Instruction::NewList(n))
        }
        "new_dict" => {
            let n_str = tokens.get(1).ok_or_else(|| format!("Line {}: new_dict requires pair count", line_num))?;
            let n = n_str.parse::<usize>().map_err(|_| format!("Line {}: invalid dict count: {}", line_num, n_str))?;
            Ok(Instruction::NewDict(n))
        }
        "new_regex" => Ok(Instruction::NewRegex),
        _ => Err(format!("Line {}: unknown operation '{}' (full: '{}')", line_num, op, original_line)),
    }
}
