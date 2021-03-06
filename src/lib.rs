use wasm_bindgen::prelude::*;
use biscuit_auth::{
    crypto::KeyPair,
    error,
    parser::parse_source,
    token::Biscuit,
    token::builder,
    token::verifier::{Verifier, VerifierLimits},
};
use log::*;
use nom::Offset;
use rand::prelude::*;
use serde::{Serialize, Deserialize};
use std::default::Default;

#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[wasm_bindgen]
extern "C" {
    // Use `js_namespace` here to bind `console.log(..)` instead of just
    // `log(..)`
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[derive(Serialize, Deserialize)]
struct BiscuitQuery {
    pub token_blocks: Vec<String>,
    pub verifier_code: Option<String>,
    pub query: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct BiscuitResult {
    pub token_blocks: Vec<Editor>,
    pub token_content: String,
    pub verifier_editor: Option<Editor>,
    pub verifier_result: Option<String>,
    pub verifier_world: Vec<Fact>,
    pub query_result: Vec<Fact>,
}

#[derive(Default, Serialize, Deserialize)]
struct Editor {
    pub errors: Vec<ParseError>,
    pub markers: Vec<Marker>,
}

#[derive(Serialize, Deserialize)]
struct Marker {
    pub ok: bool,
    pub position: SourcePosition,
}

#[derive(Serialize, Deserialize)]
struct ParseError {
    pub message: String,
    pub position: SourcePosition,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SourcePosition {
    pub line_start: usize,
    pub column_start: usize,
    pub line_end: usize,
    pub column_end: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Fact {
    pub name: String,
    pub terms: Vec<String>,
}

#[wasm_bindgen]
pub fn execute(query: &JsValue) -> JsValue {
    let query: BiscuitQuery = query.into_serde().unwrap();

    let result = execute_inner(query);

    JsValue::from_serde(&result).unwrap()
}

fn execute_inner(query: BiscuitQuery) -> BiscuitResult {
    let mut biscuit_result = BiscuitResult::default();

    info!("will generate token");

    let mut rng: StdRng = SeedableRng::seed_from_u64(0);
    let root = KeyPair::new_with_rng(&mut rng);

    let mut builder = Biscuit::builder(&root);

    let mut authority = Block::default();
    let mut blocks = Vec::new();

    let mut token_opt = None;

    if !query.token_blocks.is_empty() {
        let mut authority_editor = Editor::default();

        match parse_source(&query.token_blocks[0]) {
            Err(errors) => {
                error!("error: {:?}", errors);
                authority_editor.errors = get_parse_errors(&query.token_blocks[0], errors);
            },
            Ok((_, authority_parsed)) => {
                for (_, fact) in authority_parsed.facts.iter() {
                    builder.add_authority_fact(fact.clone()).unwrap();
                }

                for (_, rule) in authority_parsed.rules.iter() {
                    builder.add_authority_rule(rule.clone()).unwrap();
                }

                for (i, check) in authority_parsed.checks.iter() {
                    builder.add_authority_check(check.clone()).unwrap();
                    let position = get_position(&query.token_blocks[0], i);
                    authority.checks.push((position, true));
                }
            }
        }

        biscuit_result.token_blocks.push(authority_editor);

        let mut token = builder.build_with_rng(&mut rng).unwrap();

        for (i, code) in (&query.token_blocks[1..]).iter().enumerate() {
            let mut editor = Editor::default();
            let mut block = Block::default();

            let temp_keypair = KeyPair::new_with_rng(&mut rng);
            let mut builder = token.create_block();

            match parse_source(&code) {
                Err(errors) => {
                    error!("error: {:?}", errors);
                    editor.errors = get_parse_errors(&code, errors);
                },
                Ok((_, block_parsed)) => {
                    for (_, fact) in block_parsed.facts.iter() {
                        builder.add_fact(fact.clone()).unwrap();
                    }

                    for (_, rule) in block_parsed.rules.iter() {
                        builder.add_rule(rule.clone()).unwrap();
                    }

                    for (i, check) in block_parsed.checks.iter() {
                        builder.add_check(check.clone()).unwrap();
                        let position = get_position(&code, i);
                        block.checks.push((position, true));
                    }
                }
            }

            token = token
                .append_with_rng(&mut rng, &temp_keypair, builder)
                .unwrap();

            blocks.push(block);
            biscuit_result.token_blocks.push(editor);
        }

        let v = token.to_vec().unwrap();
        //self.serialized = Some(base64::encode_config(&v[..], base64::URL_SAFE));
        //self.biscuit = Some(token);
        biscuit_result.token_content = token.print();

        token_opt = Some(token);
    }

    if let Some(verifier_code) = query.verifier_code.as_ref() {
        let mut verifier = match token_opt {
            Some(token) => token.verify(root.public()).unwrap(),
            None => Verifier::new().unwrap(),
        };

        biscuit_result.verifier_editor = Some(Editor::default());
        //info!("verifier source:\n{}", &verifier_code);

        let verifier_result;

        let res = parse_source(&verifier_code);
        if let Err(errors) = res {
            biscuit_result.verifier_result = Some(format!("errors: {:?}", errors));
            error!("error: {:?}", errors);
            if let Some(ed) = biscuit_result.verifier_editor.as_mut() {
                ed.errors = get_parse_errors(&verifier_code, errors);
            }
        } else {
            let mut verifier_checks = Vec::new();
            let mut verifier_policies = Vec::new();

            let (_, parsed) = res.unwrap();

            for (_, fact) in parsed.facts.iter() {
                verifier.add_fact(fact.clone()).unwrap();
            }

            for (_, rule) in parsed.rules.iter() {
                verifier.add_rule(rule.clone()).unwrap();
            }

            for (i, check) in parsed.checks.iter() {
                verifier.add_check(check.clone()).unwrap();
                let position = get_position(&verifier_code, i);
                // checks are marked as success until they fail
                verifier_checks.push((position, true));
            }

            for (i, policy) in parsed.policies.iter() {
                verifier.add_policy(policy.clone()).unwrap();
                let position = get_position(&verifier_code, i);
                // checks are marked as success until they fail
                verifier_policies.push(position);
            }

            let mut limits = VerifierLimits::default();
            limits.max_time = std::time::Duration::from_secs(2);
            verifier_result = verifier.verify_with_limits(limits);

            let (mut facts, _, _) = verifier.dump();
            biscuit_result.verifier_world = facts.drain(..).map(|mut fact| {
                Fact {
                    name: fact.0.name,
                    terms: fact.0.ids.drain(..).map(|id| id.to_string()).collect(),
                }
            }).collect();

            match &verifier_result {
                Err(error::Token::FailedLogic(error::Logic::FailedChecks(v))) => {
                    for e in v.iter() {
                        match e {
                            error::FailedCheck::Verifier(error::FailedVerifierCheck {
                                check_id, ..
                            }) => {

                                verifier_checks[*check_id as usize].1 = false;
                            }
                            error::FailedCheck::Block(error::FailedBlockCheck {
                                block_id,
                                check_id,
                                ..
                            }) => {
                                let block = if *block_id == 0 {
                                    &mut authority
                                } else {
                                    &mut blocks[*block_id as usize - 1]
                                };
                                block.checks[*check_id as usize].1 = false;
                            }
                        }
                    }
                },
                Err(error::Token::FailedLogic(error::Logic::Deny(index))) => {
                    let position = &verifier_policies[*index];
                    if let Some(ed) = biscuit_result.verifier_editor.as_mut() {
                        ed.markers.push(Marker { ok: false, position: position.clone() });
                    }
                },
                Ok(index) => {
                    let position = &verifier_policies[*index];
                    if let Some(ed) = biscuit_result.verifier_editor.as_mut() {
                        ed.markers.push(Marker { ok: true, position: position.clone() });
                    }
                },
                _ => {},
            }

            for (position, result) in authority.checks.iter() {
                if let Some(ed) = biscuit_result.token_blocks.get_mut(0) {
                    ed.markers.push(Marker { ok: *result, position: position.clone() });
                }
            }

            for (id, block) in blocks.iter().enumerate() {
                for (position, result) in block.checks.iter() {
                    if let Some(ed) = biscuit_result.token_blocks.get_mut(id+1) {
                        ed.markers.push(Marker { ok: *result, position: position.clone() });
                    }
                }
            }

            for (position, result) in verifier_checks.iter() {
                if let Some(ed) = biscuit_result.verifier_editor.as_mut() {
                    ed.markers.push(Marker { ok: *result, position: position.clone() });
                }
            }

            biscuit_result.verifier_result = Some(match &verifier_result {
                Err(e) => format!("Error: {:?}", e),
                Ok(_) => "Success".to_string(),
            });

            if let Some(query) = query.query.as_ref() {
                log(&format!("got query content: {}", query));

                if !query.is_empty() {
                    let query_result: Result<Vec<builder::Fact>, biscuit_auth::error::Token> =
                        verifier.query(query.as_str());
                    match query_result {
                        Err(e) => {
                            log(&format!("query error: {:?}", e));
                        },
                        Ok(mut facts) => {
                            biscuit_result.query_result = facts.drain(..).map(|mut fact| {
                                Fact {
                                    name: fact.0.name,
                                    terms: fact.0.ids.drain(..).map(|id| id.to_string()).collect(),
                                }
                            }).collect();
                        }
                    }
                }
            }
        }

    }

    biscuit_result
}

#[wasm_bindgen(start)]
pub fn run_app() {
    wasm_logger::init(wasm_logger::Config::default());
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));

    unsafe { log("wasm run_app") }
}

// based on nom's convert_error
fn get_position(input: &str, span: &str) -> SourcePosition {
    let offset = input.offset(span);
    let prefix = &input.as_bytes()[..offset];

    // Count the number of newlines in the first `offset` bytes of input
    let line_start = prefix.iter().filter(|&&b| b == b'\n').count();

    // Find the line that includes the subslice:
    // find the *last* newline before the substring starts
    let line_begin = prefix
        .iter()
        .rev()
        .position(|&b| b == b'\n')
        .map(|pos| offset - pos)
        .unwrap_or(0);

    // Find the full line after that newline
    let line = input[line_begin..]
        .lines()
        .next()
        .unwrap_or(&input[line_begin..])
        .trim_end();

    // The (1-indexed) column number is the offset of our substring into that line
    let column_start = line.offset(span);

    let offset = offset + span.len();
    let prefix = &input.as_bytes()[..offset];

    // Count the number of newlines in the first `offset` bytes of input
    let line_end = prefix.iter().filter(|&&b| b == b'\n').count();

    // Find the line that includes the subslice:
    // find the *last* newline before the substring starts
    let line_begin = prefix
        .iter()
        .rev()
        .position(|&b| b == b'\n')
        .map(|pos| offset - pos)
        .unwrap_or(0);

    // Find the full line after that newline
    let line = input[line_begin..]
        .lines()
        .next()
        .unwrap_or(&input[line_begin..])
        .trim_end();

    // The (1-indexed) column number is the offset of our substring into that line
    let column_end = line.offset(&span[span.len()..]) + 1;

    SourcePosition {
        line_start,
        column_start,
        line_end,
        column_end,
    }
}

#[derive(Clone, Debug)]
struct Block {
    pub code: String,
    pub checks: Vec<(SourcePosition, bool)>,
    pub enabled: bool,
}

impl Default for Block {
    fn default() -> Self {
        Block {
            code: String::new(),
            checks: Vec::new(),
            enabled: true,
        }
    }
}

fn get_parse_errors(input: &str, errors: Vec<biscuit_auth::parser::Error>) -> Vec<ParseError> {
    let mut res = Vec::new();

    error!("got errors: {:?}", errors);
    for e in errors.iter() {
        let position = get_position(input, e.input);
        let message = e.message.as_ref().cloned().unwrap_or_else(|| format!("error: {:?}", e.code));

        error!("position for error({:?}) \"{}\": {:?}", e.code, message, position);
        res.push(ParseError { message, position });
    }

    res
}
