//! The `Term` class: terminal facts (color/tty/size) and markup as a
//! first-class operation. The color answer is THE detection the std-stream
//! writers use (`VmOptions.supports_color`: tty + `NO_COLOR` + `CLICOLOR_FORCE`),
//! never a reimplementation — `[Term]color?` true means an `#ANSI'…'` write
//! will actually render.

use crate::ansi_colorizer;
use crate::runtime::io::get_io_string;
use crate::value::NativeClassBuilder;

use std::io::IsTerminal;

pub fn build_term_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Term", Some("Object"))
        .abstract_class()
        .class_doc(
            "Terminal facts and markup rendering. `color?` answers whether styled output \
             is on — the same detection the standard-stream writers use (a terminal, \
             `NO_COLOR` unset, `CLICOLOR_FORCE` honored) — and `width`/`height` the console \
             size. `render:` turns color markup into escape codes unconditionally (for \
             output that bypasses the std streams); `strip:` turns it into plain text.\n\n\
             ```\n\
             Term.strip:#ANSI'[red]x[/]'.string     \"* -> 'x'\n\
             ```",
        )
        .class_method("color?", |vm, mc, _r, _a| {
            Ok(vm.new_bool(mc, vm.options.supports_color))
        })
        .returns("Boolean")
        .doc(
            "Whether styled output is ON for this run — stdout is a terminal, `NO_COLOR` \
             is unset (or `CLICOLOR_FORCE` set). The std-stream writers strip color \
             exactly when this is false, so it is the \"should I draw the fancy \
             version?\" question.",
        )
        .class_method("tty?", |vm, mc, _r, _a| {
            Ok(vm.new_bool(mc, std::io::stdout().is_terminal()))
        })
        .returns("Boolean")
        .doc(
            "Whether standard output is a terminal (false when piped or redirected). \
             `color?` is usually the better question — it also honors `NO_COLOR`.",
        )
        .class_method("width", |vm, mc, _r, _a| {
            Ok(match vm.options.console_width {
                Some(w) => vm.new_int(mc, w as i64),
                None => vm.new_nil(mc),
            })
        })
        .returns("Integer?")
        .doc(
            "The console width in columns, or nil when there is no terminal to \
             measure (piped output).",
        )
        .class_method("height", |vm, mc, _r, _a| {
            Ok(match vm.options.console_height {
                Some(h) => vm.new_int(mc, h as i64),
                None => vm.new_nil(mc),
            })
        })
        .returns("Integer?")
        .doc("The console height in rows, or nil when there is no terminal to measure.")
        .class_method("render:", |vm, mc, _r, args| {
            let s = get_io_string(vm, mc, args[0])?;
            Ok(vm.new_string(mc, ansi_colorizer::colorize(&s)))
        })
        .returns("String")
        .doc(
            "The markup (a String or ANSI value) rendered to ANSI escape codes \
             UNCONDITIONALLY — for output that bypasses the standard streams (a file to \
             be `cat`ed, a socket). Writing an ANSI value to `[IO]Stdout`/`Stderr` does \
             this automatically when `color?`.\n\n\
             ```\n\
             (Term.render:'[bold]x[/]').length > 1     \"* -> true\n\
             ```",
        )
        .class_method("strip:", |vm, mc, _r, args| {
            let s = get_io_string(vm, mc, args[0])?;
            Ok(vm.new_string(
                mc,
                ansi_colorizer::decolorize(&ansi_colorizer::colorize(&s)),
            ))
        })
        .returns("String")
        .doc(
            "The plain text of some markup (a String or ANSI value) — tags applied and \
             escape codes removed, exactly what a non-color write emits. The rendered \
             width of styled text is `(Term.strip:x).length`.\n\n\
             ```\n\
             Term.strip:'[red]on [[fire]![/]'     \"* -> 'on [fire]!'\n\
             ```",
        )
}
