// benches/word_motions.rs
use std::hint::black_box;
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;
use vimlet::{
    ByteOffset, Motion, MotionResolver, TextObjectKind, TextObjectScope, TextSource, WordClass,
    WordDestination,
};

fn fixtures(holdout: bool) -> Vec<(&'static str, String)> {
    if holdout {
        return vec![
            (
                "crlf",
                "fn main() {\r\n\r\n\tlet x = foo_bar(123);\r\n}\r\n".repeat(100),
            ),
            (
                "graphemes",
                "👩‍💻 e\u{301} 🇧🇷 — العربية\t漢字\u{a0}foo\n\n".repeat(100),
            ),
        ];
    }
    vec![
        (
            "source",
            include_str!("../src/motion/objects.rs").to_owned(),
        ),
        (
            "multilingual",
            "Olá, São Paulo! 日本語と Rust\t café e\u{301} 👩‍💻\n".repeat(100),
        ),
        ("long_line", "let count = some_words + 123; ".repeat(200)),
    ]
}

fn run(name: &str, adapter: &str, text: &dyn TextSource, cursors: &[usize], verify: bool) {
    for key in ["w", "b", "e", "iw", "aw"] {
        for count in [1, 10, 100] {
            let motion = match key {
                "w" => Some(Motion::Word {
                    destination: WordDestination::NextStart,
                    class: WordClass::Word,
                }),
                "b" => Some(Motion::Word {
                    destination: WordDestination::PreviousStart,
                    class: WordClass::Word,
                }),
                "e" => Some(Motion::Word {
                    destination: WordDestination::NextEnd,
                    class: WordClass::Word,
                }),
                _ => None,
            };
            let scope = if key == "iw" {
                TextObjectScope::Inner
            } else {
                TextObjectScope::Around
            };
            if verify {
                for &cursor in cursors {
                    let resolver = MotionResolver::new(text, ByteOffset(cursor));
                    if let Some(motion) = motion {
                        println!(
                            "{name}\t{adapter}\t{key}\t{count}\t{cursor}\t{:?}",
                            resolver.resolve(motion, count)
                        );
                    } else {
                        println!(
                            "{name}\t{adapter}\t{key}\t{count}\t{cursor}\t{:?}",
                            resolver.text_object(scope, TextObjectKind::Word, count)
                        );
                    }
                }
                continue;
            }
            let started = Instant::now();
            let mut operations = 0;
            while started.elapsed() < Duration::from_millis(35) {
                for &cursor in cursors {
                    let resolver =
                        MotionResolver::new(black_box(text), ByteOffset(black_box(cursor)));
                    if let Some(motion) = motion {
                        black_box(resolver.resolve(motion, count));
                    } else {
                        black_box(resolver.text_object(scope, TextObjectKind::Word, count));
                    }
                }
                operations += cursors.len();
            }
            println!(
                "{name}\t{adapter}\t{key}\t{count}\t{:.2}",
                started.elapsed().as_nanos() as f64 / operations as f64
            );
        }
    }
}

fn main() {
    let verify = std::env::args().any(|arg| arg == "--verify");
    let holdout = std::env::args().any(|arg| arg == "--holdout");
    for (name, input) in fixtures(holdout) {
        let stride = (input.graphemes(true).count() / if verify { 240 } else { 32 }).max(1);
        let mut cursors: Vec<_> = input
            .grapheme_indices(true)
            .step_by(stride)
            .map(|(index, _)| index)
            .collect();
        cursors.push(input.len());
        let rope = ropey::Rope::from_str(&input);
        run(name, "str", &input.as_str(), &cursors, verify);
        run(name, "ropey", &rope, &cursors, verify);
    }
}
