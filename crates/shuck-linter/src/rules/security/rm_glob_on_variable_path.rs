use crate::{Checker, Rule, Violation};

pub struct RmGlobOnVariablePath;

impl Violation for RmGlobOnVariablePath {
    fn rule() -> Rule {
        Rule::RmGlobOnVariablePath
    }

    fn message(&self) -> String {
        "recursive `rm` on a variable path can delete more than intended".to_owned()
    }
}

pub fn rm_glob_on_variable_path(checker: &mut Checker) {
    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("rm"))
        .filter(|fact| {
            !fact
                .zsh_options()
                .is_some_and(|options| options.glob.is_definitely_off())
        })
        .filter_map(|fact| fact.options().rm())
        .flat_map(|rm| rm.dangerous_path_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all(spans, || RmGlobOnVariablePath);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_globbed_trimmed_parameter_expansion_paths() {
        let source = "#!/bin/sh\ndir=/tmp/\nrm -rf \"${dir%/}\"/*\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RmGlobOnVariablePath),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::RmGlobOnVariablePath);
        assert_eq!(diagnostics[0].span.slice(source), "\"${dir%/}\"/*");
    }

    #[test]
    fn ignores_safe_rm_forms_without_globbed_variable_target() {
        let source = "#!/bin/bash\ndir=/tmp\nfallback=\nrm -rf -- \"$dir\"\nrm -rf /var/tmp/*\nrm -rf \"$dir\"/cache\nrm -rf \"${fallback:-/tmp}\"/*\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RmGlobOnVariablePath),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_guarded_parameter_segments_that_shellcheck_accepts() {
        let source = "#!/bin/bash\ndir=/tmp\nrm -rf \"$dir/${dev:-does_not_exist}\"\nrm -rf \"${NVM_DIR}/${TEST_VERSION:?}\" .nvmrc\nrm -rf \"${foo:-\"$bar/baz\"}/$1\"/\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RmGlobOnVariablePath),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_indirect_expansion_rm_targets() {
        let source = "#!/bin/bash\nroot_ref=HOME\nrm -rf \"${!root_ref}\"/*\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RmGlobOnVariablePath),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::RmGlobOnVariablePath);
        assert_eq!(diagnostics[0].span.slice(source), "\"${!root_ref}\"/*");
    }

    #[test]
    fn reports_absolute_system_prefixes_with_dynamic_tails() {
        let source = "#!/bin/bash\nPRGNAM=demo\nREMOVE='*.exe *.dll'\nrm -rf /usr/share/$PRGNAM\nfor item in $REMOVE; do\n  rm -rf /usr/share/$PRGNAM/$item\ndone\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RmGlobOnVariablePath),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["/usr/share/$PRGNAM", "/usr/share/$PRGNAM/$item"]
        );
    }

    #[test]
    fn reports_variable_roots_with_explicit_trailing_separators() {
        let source =
            "#!/bin/bash\nPKG=/pkg\nPACKAGE=/archive\nrm -rf $PKG/\nrm -rf \"${PACKAGE}/\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RmGlobOnVariablePath),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$PKG/", "\"${PACKAGE}/\""]
        );
    }

    #[test]
    fn preserves_shellcheck_duplicate_hits_for_brace_expansions() {
        let source = "#!/bin/bash\nPKG=/pkg\nDESTDIR=/dest\nSYSROOT=/target\nrm -rf $PKG/usr/{bin,include,share}\nrm -rf ${DESTDIR}/${SYSROOT}/{sbin,etc,var,libexec}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RmGlobOnVariablePath),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$PKG/usr/{bin,include,share}",
                "$PKG/usr/{bin,include,share}",
                "${DESTDIR}/${SYSROOT}/{sbin,etc,var,libexec}",
                "${DESTDIR}/${SYSROOT}/{sbin,etc,var,libexec}",
            ]
        );
    }

    #[test]
    fn reports_variable_paths_that_collapse_into_system_subtrees() {
        let source = "#!/bin/bash\nPKG=/pkg\nPRGNAM=demo\nDESTDIR=/dest\nPYDIR=/py\nSUFFIX=\nrm -rf $PKG/usr\nrm -rf $PKG/usr/share/$PRGNAM\nrm -rf \"$DESTDIR\"/usr\nrm -rf $PKG/usr/{bin,include,libexec,man,share}\nrm -rf \"$PKG/$PYDIR/usr\"\nrm -rf $PKG/$PYDIR/*\nrm -rf \"$DESTDIR\"/${PRGNAM}*\nrm -rf \"$DESTDIR\"/usr${SUFFIX}\nrm -rf \"$DESTDIR\"/usr${SUFFIX}/$PRGNAM\nrm -rf \"$DESTDIR\"/usr/${PRGNAM}*\nrm -rf \"$DESTDIR\"/lib/${PRGNAM}*\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RmGlobOnVariablePath),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$PKG/usr",
                "$PKG/usr/share/$PRGNAM",
                "\"$DESTDIR\"/usr",
                "$PKG/usr/{bin,include,libexec,man,share}",
                "$PKG/usr/{bin,include,libexec,man,share}",
                "\"$PKG/$PYDIR/usr\"",
                "$PKG/$PYDIR/*",
                "\"$DESTDIR\"/${PRGNAM}*",
                "\"$DESTDIR\"/usr${SUFFIX}",
                "\"$DESTDIR\"/usr${SUFFIX}/$PRGNAM",
                "\"$DESTDIR\"/usr/${PRGNAM}*",
                "\"$DESTDIR\"/lib/${PRGNAM}*",
            ]
        );
    }

    #[test]
    fn ignores_component_globs_that_do_not_target_known_system_roots() {
        let source = "#!/bin/bash\nPKG=/pkg\nPYDIR=/py\nDESTDIR=/dest\nPRGNAM=demo\nLIBDIRSUFFIX=64\nrm -rf $PKG/$PYDIR/lib*\nrm -rf \"$DESTDIR\"/lib*\nrm -rf \"$DESTDIR\"/opt\nrm -rf \"$DESTDIR\"/opt/$PRGNAM\nrm -rf $PKG/usr/share/doc\nrm -rf $PKG/usr/share/icons\nrm -rf $PKG/usr/doc/$PRGNAM\nrm -rf $PKG/usr/lib${LIBDIRSUFFIX}/*.la\nrm -rf $PKG/usr/share/$PRGNAM/icons\nrm -rf $PKG/opt/$PRGNAM/bin\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RmGlobOnVariablePath),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
