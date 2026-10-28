//! # Command grammar
//!
//! **Purpose:** read vi's little language — `"a3dw`, `ci(`, `5G`, `gUiw` —
//! out of a run of keys.
//!
//! **Responsibility:** a pure function from the keys typed so far to one of
//! three answers: wait for more, here is the action, or that was not a
//! command. Keeping it stateless (the caller holds the keys) means a count, a
//! register and an operator compose without a state machine per combination,
//! and a macro or a key mapping replays through exactly the same code.
//!
//! The grammar is
//!
//! ```text
//! command  := ["x] [count] ( goal | operator [count] target | simple )
//! target   := operator-key            (the operator doubled: dd, gUU, gcc)
//!           | (i | a) object-key      (text objects)
//!           | goal                    (any motion)
//! ```
//!
//! **Public API:** [`Parse`], [`Context`], [`normal`], [`visual`].

use crossterm::event::{KeyCode, KeyEvent};

use super::{
    Action, CaseChange, FindSpec, FoldCommand, Goal, Operator, ScreenRow, Target, is_plain,
};
use crate::app::mode::Mode;
use crate::editor::cursor::Motion;
use crate::editor::textobject::TextObject;

/// What the keys typed so far amount to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parse {
    /// A prefix of some command; keep the keys and wait.
    Incomplete,
    /// A complete command.
    Done(Action),
    /// Not a command; throw the keys away.
    Invalid,
}

/// The little state the grammar needs from outside the keys themselves.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    /// The last `f`, `t`, `F` or `T`, which `;` and `,` repeat.
    pub last_find: Option<FindSpec>,
    /// Whether a macro is being recorded, which turns a lone `q` into "stop".
    pub recording: bool,
    /// The mode the keys were typed in.
    pub mode: Mode,
}

/// Why parsing stopped early.
enum Stop {
    Incomplete,
    Invalid,
}

type Step<T> = Result<T, Stop>;

/// One key as far as the grammar is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    Char(char),
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Other,
}

fn token(key: &KeyEvent) -> Token {
    if !is_plain(key.modifiers) {
        return Token::Other;
    }
    match key.code {
        KeyCode::Char(ch) => Token::Char(ch),
        KeyCode::Left => Token::Left,
        KeyCode::Right => Token::Right,
        KeyCode::Up => Token::Up,
        KeyCode::Down => Token::Down,
        KeyCode::Home => Token::Home,
        KeyCode::End => Token::End,
        _ => Token::Other,
    }
}

struct Parser<'a> {
    keys: &'a [KeyEvent],
    at: usize,
    context: Context,
}

/// Parse keys typed in normal mode.
#[must_use]
pub fn normal(keys: &[KeyEvent], context: Context) -> Parse {
    run(keys, context, Parser::normal)
}

/// Parse keys typed in one of the visual modes.
#[must_use]
pub fn visual(keys: &[KeyEvent], context: Context) -> Parse {
    run(keys, context, Parser::visual)
}

fn run<'k>(
    keys: &'k [KeyEvent],
    context: Context,
    rule: fn(&mut Parser<'k>) -> Step<Action>,
) -> Parse {
    let mut parser = Parser {
        keys,
        at: 0,
        context,
    };
    match rule(&mut parser) {
        Ok(action) if parser.at == keys.len() => Parse::Done(action),
        Ok(_) | Err(Stop::Invalid) => Parse::Invalid,
        Err(Stop::Incomplete) => Parse::Incomplete,
    }
}

impl Parser<'_> {
    fn next(&mut self) -> Step<Token> {
        let key = self.keys.get(self.at).ok_or(Stop::Incomplete)?;
        self.at += 1;
        Ok(token(key))
    }

    fn peek(&self) -> Option<Token> {
        self.keys.get(self.at).map(token)
    }

    fn char(&mut self) -> Step<char> {
        match self.next()? {
            Token::Char(ch) => Ok(ch),
            _ => Err(Stop::Invalid),
        }
    }

    /// `"x` — which register the command reads or writes.
    fn register(&mut self) -> Step<Option<char>> {
        if self.peek() != Some(Token::Char('"')) {
            return Ok(None);
        }
        self.at += 1;
        let name = self.char()?;
        if is_register(name) {
            Ok(Some(name))
        } else {
            Err(Stop::Invalid)
        }
    }

    /// Digits before a command. A leading `0` is the motion, not a count.
    fn count(&mut self) -> Step<Option<usize>> {
        let mut count: Option<usize> = None;
        while let Some(Token::Char(ch)) = self.peek() {
            let Some(digit) = ch.to_digit(10) else {
                break;
            };
            if count.is_none() && digit == 0 {
                break;
            }
            self.at += 1;
            let value = count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(digit as usize);
            count = Some(value.min(100_000));
        }
        Ok(count)
    }

    /// A motion, mark or screen position, given its first key.
    fn goal(&mut self, token: Token) -> Step<Option<Goal>> {
        let motion = match token {
            Token::Left | Token::Char('h') => Motion::Left,
            Token::Right | Token::Char('l' | ' ') => Motion::Right,
            Token::Up | Token::Char('k') => Motion::Up(1),
            Token::Down | Token::Char('j') => Motion::Down(1),
            Token::Home | Token::Char('0') => Motion::LineStart,
            Token::End | Token::Char('$') => Motion::LineEnd,
            Token::Char('w') => Motion::WordForward,
            Token::Char('b') => Motion::WordBackward,
            Token::Char('e') => Motion::WordEnd,
            Token::Char('W') => Motion::BigWordForward,
            Token::Char('B') => Motion::BigWordBackward,
            Token::Char('E') => Motion::BigWordEnd,
            Token::Char('^' | '_') => Motion::LineFirstNonBlank,
            Token::Char('G') => Motion::DocEnd,
            Token::Char('}') => Motion::ParagraphForward,
            Token::Char('{') => Motion::ParagraphBackward,
            Token::Char('%') => Motion::MatchingBracket,
            Token::Char(key @ ('f' | 't' | 'F' | 'T')) => {
                let target = self.char()?;
                return Ok(Some(Goal::Find(FindSpec {
                    target,
                    forward: key.is_lowercase(),
                    till: matches!(key, 't' | 'T'),
                })));
            }
            Token::Char(key @ (';' | ',')) => {
                let last = self.context.last_find.ok_or(Stop::Invalid)?;
                let spec = if key == ',' { last.reversed() } else { last };
                return Ok(Some(Goal::Repeat(spec)));
            }
            Token::Char(key @ ('\'' | '`')) => {
                let name = self.char()?;
                return Ok(Some(Goal::Mark {
                    name,
                    exact: key == '`',
                }));
            }
            Token::Char('H') => return Ok(Some(Goal::Screen(ScreenRow::Top))),
            Token::Char('M') => return Ok(Some(Goal::Screen(ScreenRow::Middle))),
            Token::Char('L') => return Ok(Some(Goal::Screen(ScreenRow::Bottom))),
            _ => return Ok(None),
        };
        Ok(Some(Goal::Motion(motion)))
    }

    /// The goals reached through `g`: `gg`, `ge`, `gh`, `gl` and `gs`.
    fn g_goal(key: char) -> Option<Goal> {
        Some(Goal::Motion(match key {
            'g' => Motion::DocStart,
            'e' => Motion::DocEnd,
            'h' => Motion::LineStart,
            'l' => Motion::LineEnd,
            's' => Motion::LineFirstNonBlank,
            _ => return None,
        }))
    }

    fn normal(&mut self) -> Step<Action> {
        let register = self.register()?;
        let count = self.count()?;
        let action = self.normal_body(count)?;
        Ok(with_register(register, action))
    }

    fn normal_body(&mut self, count: Option<usize>) -> Step<Action> {
        let n = count.unwrap_or(1);
        let token = self.next()?;
        if let Some(operator) = Operator::from_key(token_char(token)) {
            return self.operator_target(operator, count);
        }
        if let Some(goal) = self.goal(token)? {
            return Ok(movement(goal, count));
        }
        let Token::Char(key) = token else {
            return Err(Stop::Invalid);
        };
        Ok(match key {
            'i' => Action::EnterMode(Mode::Insert),
            'a' => Action::AppendAfter,
            'I' => Action::InsertAtLineStart,
            'A' => Action::AppendAtLineEnd,
            'o' => Action::OpenLineBelow,
            'O' => Action::OpenLineAbove,
            'v' => Action::EnterMode(Mode::Visual),
            'V' => Action::EnterMode(Mode::VisualLine),
            ':' => Action::EnterMode(Mode::Command),
            'x' => operate(Operator::Delete, Goal::Motion(Motion::Right), n),
            'X' => operate(Operator::Delete, Goal::Motion(Motion::Left), n),
            's' => operate(Operator::Change, Goal::Motion(Motion::Right), n),
            'S' => Action::Operate {
                operator: Operator::Change,
                target: Target::Lines,
                count: n,
            },
            'C' => operate(Operator::Change, Goal::Motion(Motion::LineEnd), n),
            'D' => operate(Operator::Delete, Goal::Motion(Motion::LineEnd), n),
            'Y' => Action::Operate {
                operator: Operator::Yank,
                target: Target::Lines,
                count: n,
            },
            'p' => Action::Put {
                before: false,
                count: n,
            },
            'P' => Action::Put {
                before: true,
                count: n,
            },
            'u' => repeat(count, Action::Undo),
            '.' => Action::RepeatChange(count),
            'r' => Action::ReplaceChar {
                ch: self.char()?,
                count: n,
            },
            '~' => Action::ToggleCaseChars(n),
            'J' => Action::Join {
                count: n,
                spaces: true,
            },
            'm' => Action::SetMark(self.char()?),
            'q' if self.context.recording => Action::StopMacro,
            'q' => {
                let name = self.char()?;
                if !is_register(name) || matches!(name, '_' | '+' | '*' | '"') {
                    return Err(Stop::Invalid);
                }
                Action::RecordMacro(name)
            }
            '@' => Action::PlayMacro {
                register: self.char()?,
                count: n,
            },
            '*' => Action::SearchWord {
                forward: true,
                whole: true,
            },
            '#' => Action::SearchWord {
                forward: false,
                whole: true,
            },
            'n' => repeat(count, Action::SearchRepeat { forward: true }),
            'N' => repeat(count, Action::SearchRepeat { forward: false }),
            '/' => Action::SearchStart { forward: true },
            '?' => Action::SearchStart { forward: false },
            'Z' => match self.char()? {
                'Z' => Action::RunCommand("x".to_string()),
                'Q' => Action::RunCommand("q!".to_string()),
                _ => return Err(Stop::Invalid),
            },
            'z' => self.z_command()?,
            'g' => self.g_command(count)?,
            '[' | ']' => self.bracket_command(key == ']')?,
            _ => return Err(Stop::Invalid),
        })
    }

    /// `]c`, `[c` and the other "next/previous thing" pairs.
    fn bracket_command(&mut self, forward: bool) -> Step<Action> {
        Ok(match self.char()? {
            'c' => Action::JumpHunk { forward },
            _ => return Err(Stop::Invalid),
        })
    }

    /// `z` commands: scroll the caret's line to a place on screen.
    fn z_command(&mut self) -> Step<Action> {
        Ok(match self.char()? {
            'z' | '.' => Action::ScrollCaret(ScreenRow::Middle),
            't' => Action::ScrollCaret(ScreenRow::Top),
            'b' | '-' => Action::ScrollCaret(ScreenRow::Bottom),
            'a' => Action::Fold(FoldCommand::Toggle),
            'c' => Action::Fold(FoldCommand::Close),
            'o' => Action::Fold(FoldCommand::Open),
            'R' => Action::Fold(FoldCommand::OpenAll),
            'M' => Action::Fold(FoldCommand::CloseAll),
            _ => return Err(Stop::Invalid),
        })
    }

    /// `g` commands that are neither operators nor motions.
    fn g_command(&mut self, count: Option<usize>) -> Step<Action> {
        let n = count.unwrap_or(1);
        let key = self.char()?;
        if let Some(operator) = Operator::from_g_key(key) {
            return self.operator_target(operator, count);
        }
        if let Some(goal) = Self::g_goal(key) {
            return Ok(movement(goal, count));
        }
        Ok(match key {
            'J' => Action::Join {
                count: n,
                spaces: false,
            },
            'v' => Action::Reselect,
            '-' => Action::UndoEarlier(n),
            '+' => Action::UndoLater(n),
            '*' => Action::SearchWord {
                forward: true,
                whole: false,
            },
            '#' => Action::SearchWord {
                forward: false,
                whole: false,
            },
            't' => Action::CycleBuffer { forward: true },
            'T' => Action::CycleBuffer { forward: false },
            'p' => Action::Put {
                before: false,
                count: n,
            },
            'P' => Action::Put {
                before: true,
                count: n,
            },
            _ => return Err(Stop::Invalid),
        })
    }

    /// Everything after an operator: a second count and what it acts on.
    fn operator_target(&mut self, operator: Operator, count: Option<usize>) -> Step<Action> {
        let second = self.count()?;
        let n = count.unwrap_or(1).saturating_mul(second.unwrap_or(1));
        let token = self.next()?;
        let key = token_char(token);

        if key.is_some() && key == operator.doubled() {
            return Ok(Action::Operate {
                operator,
                target: Target::Lines,
                count: n,
            });
        }
        if let Some(inner @ ('i' | 'a')) = key {
            let object = TextObject::from_key(self.char()?).ok_or(Stop::Invalid)?;
            return Ok(Action::Operate {
                operator,
                target: Target::Object {
                    object,
                    inner: inner == 'i',
                },
                count: n,
            });
        }
        if key == Some('g') {
            let next = self.char()?;
            // `gugu`, `gUgU`, `g~g~` and `gcgc` double the whole operator.
            if Operator::from_g_key(next) == Some(operator) {
                return Ok(Action::Operate {
                    operator,
                    target: Target::Lines,
                    count: n,
                });
            }
            let goal = Self::g_goal(next).ok_or(Stop::Invalid)?;
            return Ok(operate_goal(operator, goal, count, second));
        }
        let goal = self.goal(token)?.ok_or(Stop::Invalid)?;
        Ok(operate_goal(operator, goal, count, second))
    }

    fn visual(&mut self) -> Step<Action> {
        let register = self.register()?;
        let count = self.count()?;
        let action = self.visual_body(count)?;
        Ok(with_register(register, action))
    }

    fn visual_body(&mut self, count: Option<usize>) -> Step<Action> {
        let n = count.unwrap_or(1);
        let token = self.next()?;
        if let Some(goal) = self.goal(token)? {
            return Ok(movement(goal, count));
        }
        let Token::Char(key) = token else {
            return Err(Stop::Invalid);
        };
        let selection = |operator| Action::OperateSelection { operator, count: n };
        Ok(match key {
            'i' | 'a' => {
                let object = TextObject::from_key(self.char()?).ok_or(Stop::Invalid)?;
                Action::SelectObject {
                    object,
                    inner: key == 'i',
                    count: n,
                }
            }
            'd' | 'x' | 'X' | 'D' => selection(Operator::Delete),
            'y' | 'Y' => selection(Operator::Yank),
            'c' | 's' | 'C' | 'S' | 'R' => selection(Operator::Change),
            '>' => selection(Operator::Indent),
            '<' => selection(Operator::Dedent),
            '~' => selection(Operator::Case(CaseChange::Toggle)),
            'u' => selection(Operator::Case(CaseChange::Lower)),
            'U' => selection(Operator::Case(CaseChange::Upper)),
            'J' => Action::JoinSelection { spaces: true },
            'r' => Action::ReplaceSelection(self.char()?),
            'p' | 'P' => Action::Paste,
            'o' | 'O' => Action::SwapAnchor,
            'I' => Action::BlockInsert { append: false },
            'A' => Action::BlockInsert { append: true },
            'v' => Action::EnterMode(toggle(self.context.mode, Mode::Visual)),
            'V' => Action::EnterMode(toggle(self.context.mode, Mode::VisualLine)),
            ':' => Action::EnterMode(Mode::Command),
            '*' => Action::SearchWord {
                forward: true,
                whole: true,
            },
            'g' => {
                let next = self.char()?;
                if let Some(goal) = Self::g_goal(next) {
                    return Ok(movement(goal, count));
                }
                match next {
                    'c' => selection(Operator::Comment),
                    'u' => selection(Operator::Case(CaseChange::Lower)),
                    'U' => selection(Operator::Case(CaseChange::Upper)),
                    '~' => selection(Operator::Case(CaseChange::Toggle)),
                    'J' => Action::JoinSelection { spaces: false },
                    _ => return Err(Stop::Invalid),
                }
            }
            'z' => self.z_command()?,
            _ => return Err(Stop::Invalid),
        })
    }
}

/// Leaving a visual mode by pressing its own key again, vi style.
fn toggle(current: Mode, pressed: Mode) -> Mode {
    if current == pressed {
        Mode::Normal
    } else {
        pressed
    }
}

fn token_char(token: Token) -> Option<char> {
    match token {
        Token::Char(ch) => Some(ch),
        _ => None,
    }
}

/// Whether `name` can follow `"`.
#[must_use]
pub fn is_register(name: char) -> bool {
    name.is_ascii_alphanumeric()
        || matches!(name, '"' | '-' | '_' | '+' | '*' | '/' | ':' | '%' | '.')
}

fn with_register(register: Option<char>, action: Action) -> Action {
    match register {
        Some(register) => Action::WithRegister {
            register,
            action: Box::new(action),
        },
        None => action,
    }
}

fn repeat(count: Option<usize>, action: Action) -> Action {
    match count {
        Some(count) if count > 1 => Action::Repeat {
            count,
            action: Box::new(action),
        },
        _ => action,
    }
}

/// A goal reached by moving, with as cheap an action as the count allows.
fn movement(goal: Goal, count: Option<usize>) -> Action {
    if let Goal::Motion(motion) = goal {
        match count {
            None => return Action::Move(motion),
            Some(count) => {
                if let Some(scaled) = motion.scaled(count) {
                    return Action::Move(scaled);
                }
            }
        }
    }
    Action::Goto {
        goal,
        count: count.unwrap_or(1),
    }
}

fn operate(operator: Operator, goal: Goal, count: usize) -> Action {
    Action::Operate {
        operator,
        target: Target::Goal(goal),
        count,
    }
}

/// `3dw` and `d3w` move three words; `dG` and `d5G` mean a line, not a count.
fn operate_goal(
    operator: Operator,
    goal: Goal,
    first: Option<usize>,
    second: Option<usize>,
) -> Action {
    let count = match (first, second) {
        (None, None) => None,
        (first, second) => Some(first.unwrap_or(1) * second.unwrap_or(1)),
    };
    let goal = match (goal, count) {
        (Goal::Motion(motion @ (Motion::DocEnd | Motion::DocStart)), Some(count)) => {
            Goal::Motion(motion.scaled(count).unwrap_or(motion))
        }
        (goal, _) => goal,
    };
    let count = match goal {
        Goal::Motion(Motion::ToLine(_)) => 1,
        _ => count.unwrap_or(1),
    };
    operate(operator, goal, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn keys(text: &str) -> Vec<KeyEvent> {
        text.chars()
            .map(|ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .collect()
    }

    fn parse(text: &str) -> Parse {
        normal(&keys(text), Context::default())
    }

    fn done(text: &str) -> Action {
        match parse(text) {
            Parse::Done(action) => action,
            other => panic!("{text:?} parsed as {other:?}"),
        }
    }

    #[test]
    fn a_partial_command_waits_for_more_keys() {
        for prefix in ["d", "3", "\"", "\"a", "di", "g", "f", "2d3", "gu"] {
            assert_eq!(parse(prefix), Parse::Incomplete, "{prefix:?}");
        }
    }

    #[test]
    fn nonsense_is_rejected() {
        assert_eq!(parse("dQ"), Parse::Invalid);
        assert_eq!(parse("diQ"), Parse::Invalid);
    }

    #[test]
    fn counts_multiply_across_the_operator() {
        assert_eq!(
            done("2d3w"),
            Action::Operate {
                operator: Operator::Delete,
                target: Target::Goal(Goal::Motion(Motion::WordForward)),
                count: 6,
            }
        );
    }

    #[test]
    fn a_doubled_operator_acts_on_lines() {
        for (text, operator) in [
            ("dd", Operator::Delete),
            ("yy", Operator::Yank),
            ("cc", Operator::Change),
            (">>", Operator::Indent),
            ("gcc", Operator::Comment),
            ("gUU", Operator::Case(CaseChange::Upper)),
            ("gugu", Operator::Case(CaseChange::Lower)),
        ] {
            assert_eq!(
                done(text),
                Action::Operate {
                    operator,
                    target: Target::Lines,
                    count: 1,
                },
                "{text}"
            );
        }
    }

    #[test]
    fn text_objects_follow_an_operator() {
        assert_eq!(
            done("ci("),
            Action::Operate {
                operator: Operator::Change,
                target: Target::Object {
                    object: TextObject::Pair('(', ')'),
                    inner: true,
                },
                count: 1,
            }
        );
    }

    #[test]
    fn registers_wrap_the_command() {
        assert_eq!(
            done("\"ayy"),
            Action::WithRegister {
                register: 'a',
                action: Box::new(Action::Operate {
                    operator: Operator::Yank,
                    target: Target::Lines,
                    count: 1,
                }),
            }
        );
    }

    #[test]
    fn a_count_before_a_line_motion_names_the_line() {
        assert_eq!(done("5G"), Action::Move(Motion::ToLine(4)));
        assert_eq!(done("3gg"), Action::Move(Motion::ToLine(2)));
        assert_eq!(done("4j"), Action::Move(Motion::Down(4)));
        assert_eq!(done("G"), Action::Move(Motion::DocEnd));
        assert_eq!(done("0"), Action::Move(Motion::LineStart));
    }

    #[test]
    fn find_and_its_repeat() {
        let spec = FindSpec {
            target: 'x',
            forward: true,
            till: true,
        };
        assert_eq!(
            done("tx"),
            Action::Goto {
                goal: Goal::Find(spec),
                count: 1
            }
        );
        let context = Context {
            last_find: Some(spec),
            ..Context::default()
        };
        assert_eq!(
            normal(&keys(","), context),
            Parse::Done(Action::Goto {
                goal: Goal::Repeat(spec.reversed()),
                count: 1
            })
        );
        assert_eq!(parse(";"), Parse::Invalid);
    }

    #[test]
    fn q_starts_and_stops_a_recording() {
        assert_eq!(done("qa"), Action::RecordMacro('a'));
        let recording = Context {
            recording: true,
            ..Context::default()
        };
        assert_eq!(
            normal(&keys("q"), recording),
            Parse::Done(Action::StopMacro)
        );
    }

    #[test]
    fn visual_operators_act_on_the_selection() {
        let context = Context {
            mode: Mode::Visual,
            ..Context::default()
        };
        assert_eq!(
            visual(&keys("d"), context),
            Parse::Done(Action::OperateSelection {
                operator: Operator::Delete,
                count: 1
            })
        );
        assert_eq!(
            visual(&keys("iw"), context),
            Parse::Done(Action::SelectObject {
                object: TextObject::Word,
                inner: true,
                count: 1
            })
        );
        assert_eq!(
            visual(&keys("v"), context),
            Parse::Done(Action::EnterMode(Mode::Normal))
        );
    }
}
