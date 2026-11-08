use super::*;
#[test]
fn jobserver_flags_survive_without_descriptors_or_credential_assignments() {
    let vars = inherited([
        (
            "MAKEFLAGS".into(),
            "-- ARCHON_TEST_SECRET=sk-test-355".into(),
        ),
        ("MFLAGS".into(), "--jobserver-auth=3,4 --silent".into()),
        (
            "GNUMAKEFLAGS".into(),
            "--jobserver-auth=fifo:/tmp/archon-jobserver -- X=keep".into(),
        ),
    ]);
    assert!(!vars.iter().any(|(key, _)| key == "MAKEFLAGS"));
    assert!(
        vars.iter()
            .any(|(key, value)| key == "MFLAGS" && value == "--silent")
    );
    assert!(vars.iter().any(|(key, value)| key == "GNUMAKEFLAGS"
        && value == "--jobserver-auth=fifo:/tmp/archon-jobserver"));
}

#[test]
fn build_flags_keep_options_and_drop_all_assignments() {
    for (input, expected) in [
        (
            "-j4 --jobserver-auth=3,4 -- FOO=bar SECRET_X=y",
            Some("-j4 --jobserver-auth=3,4"),
        ),
        (
            "--jobserver-auth=gmake_semaphore_22416 -j4",
            Some("--jobserver-auth=gmake_semaphore_22416 -j4"),
        ),
        (
            "--jobserver-fds=gmake_semaphore_42",
            Some("--jobserver-fds=gmake_semaphore_42"),
        ),
        (
            "--jobserver-auth=gmake_semaphore_ --jobserver-fds=gmake_semaphore_x -s",
            Some("-s"),
        ),
        (
            "-Otarget --sync-mutex=fnm:/tmp/GmAbC123",
            Some("-Otarget --sync-mutex=fnm:/tmp/GmAbC123"),
        ),
        ("--sync-mutex=0x1a4", Some("--sync-mutex=0x1a4")),
        ("--sync-mutex=fnm:rel -s", Some("-s")),
        ("--sync-mutex=0xZZ -s", Some("-s")),
        ("--sync-mutex=0x --sync-mutex=fnm:/tmp/a:b -s", Some("-s")),
        ("--debug=all -k", Some("--debug=all -k")),
        ("--debug=Basic,V", Some("--debug=Basic,V")),
        ("--debug=bSecret -s", Some("-s")),
        ("I /tmp -I /usr", Some("I /tmp -I /usr")),
        ("-IC:/inc -s", Some("-IC:/inc -s")),
        ("-I/opt/Example", Some("-I/opt/Example")),
        (
            "ks -I/Volumes/Externalwork/mk",
            Some("ks -I/Volumes/Externalwork/mk"),
        ),
        ("-IC:/inc -IE:/inc -j4", Some("-IC:/inc -IE:/inc -j4")),
        ("-I /usr/include/c++ -s", Some("-I /usr/include/c++ -s")),
        ("-I/opt/~user@host,inc -s", Some("-I/opt/~user@host,inc -s")),
        ("-kE TOKEN=abc -s", Some("-s")),
        ("-sEk -k", Some("-k")),
        ("kE TOKEN=abc -s", Some("-s")),
        ("sEk -k", Some("-k")),
        ("-I/a=b -s", Some("-s")),
        ("-I$HOME -s", Some("-s")),
        ("-I TOKEN=abc -s", Some("-s")),
        ("-I -s", Some("-I -s")),
        ("-kI /tmp -sI C:/inc", Some("-kI /tmp -sI C:/inc")),
        ("kI /tmp", Some("kI /tmp")),
        ("-I foo\\ bar -s", Some("-s")),
        ("-I é -s", Some("-s")),
        ("-I E -s", Some("-I E -s")),
        ("-I/tmp/I TOKEN=abc -s", Some("-I/tmp/I -s")),
        (
            "--jobserver-auth=fifo:C:/tmp --sync-mutex=fnm:C:/tmp -s",
            Some("-s"),
        ),
        ("-sl 2.5", Some("-sl 2.5")),
        ("kj", Some("kj")),
        ("kj 4", Some("kj 4")),
        ("sl 2.5", Some("sl 2.5")),
        ("-kj 4", Some("-kj 4")),
        ("--ghp_RealToken123 -s", Some("-s")),
        ("--jobserver-a -s", Some("-s")),
        ("--load -s", Some("-s")),
        ("--debug=ghpSecretLetters -s", Some("-s")),
        ("--debug=basic,j", Some("--debug=basic,j")),
        (
            "--debug=a,b,v,i,j,m,n,p,w,basic,verbose,implicit,jobs,makefile,none,print,why",
            Some("--debug=a,b,v,i,j,m,n,p,w,basic,verbose,implicit,jobs,makefile,none,print,why"),
        ),
        ("--debug= --debug=a, --debug=ab -s", Some("-s")),
        ("--jobserver-auth=fifo:sk_live_abc123 -s", Some("-s")),
        (
            "--jobserver-auth=fifo:/tmp/GMfifo123",
            Some("--jobserver-auth=fifo:/tmp/GMfifo123"),
        ),
        (
            "--jobserver-auth=fifo:/tmp/a:b --jobserver-auth=fifo:/tmp/a@b -s",
            Some("-s"),
        ),
        ("-I/usr/include -s", Some("-I/usr/include -s")),
        ("-Ifoo=bar -s", Some("-s")),
        ("-I -Ix:y -s", Some("-I -Ix:y -s")),
        ("--shuffle=reverse", Some("--shuffle=reverse")),
        (
            "--shuffle=random --shuffle=none --shuffle=identity --shuffle=123",
            Some("--shuffle=random --shuffle=none --shuffle=identity --shuffle=123"),
        ),
        ("--shuffle=SECRET --shuffle=12x --shuffle= -s", Some("-s")),
        ("--no-print-directory -w", Some("--no-print-directory -w")),
        (
            "-kBsj4 -sl2.5 -kOline -sI/tmp/include",
            Some("-kBsj4 -sl2.5 -kOline -sI/tmp/include"),
        ),
        (
            "-x -kj4s -sl2.5k -Olines -Ifoo@bar -s",
            Some("-Ifoo@bar -s"),
        ),
        ("-sl 2.5.6 -kj 4x -s", Some("-sl -kj -s")),
        ("kjs -s", Some("-s")),
        ("-é -s", Some("-s")),
        (
            "--always-make --environment-overrides --ignore-errors --keep-going --no-keep-going --stop --just-print --dry-run --recon --print-data-base --question --no-builtin-rules --no-builtin-variables --silent --quiet --no-silent --touch --trace --print-directory --no-print-directory --warn-undefined-variables --check-symlink-times --debug --jobs --load-average --max-load --output-sync --shuffle",
            Some(
                "--always-make --environment-overrides --ignore-errors --keep-going --no-keep-going --stop --just-print --dry-run --recon --print-data-base --question --no-builtin-rules --no-builtin-variables --silent --quiet --no-silent --touch --trace --print-directory --no-print-directory --warn-undefined-variables --check-symlink-times --debug --jobs --load-average --max-load --output-sync --shuffle",
            ),
        ),
        ("--debug=TOKEN --SERVICE_AUTH=x -s", Some("-s")),
        ("-l.5 -l5. -s", Some("-s")),
        ("ks -- FOO=bar", Some("ks")),
        ("ks -- ", Some("ks")),
        ("-j 4 -k -s --silent -w", Some("-j 4 -k -s --silent -w")),
        (
            "-j8 FOO=bar SECRET_X=y SERVICE_AUTH=z --jobserver-fds=3,4",
            Some("-j8 --jobserver-fds=3,4"),
        ),
        ("--debug=SECRET_X=y", None),
        ("--jobserver-auth=fifo:/tmp/x\\ TOKEN=abc", None),
        ("-O=a=b", None),
        ("--ev TOKEN=abc -s", Some("-s")),
        ("--eva=TOKEN -k", Some("-k")),
        ("--eva=TOKENE -k", Some("-k")),
        ("--ev -k -s", Some("-s")),
        ("--ev=TOKEN -s", Some("-s")),
        ("--eva TOKEN=abc -k", Some("-k")),
        ("--jobserver-a=3,4 --load=2 -s", Some("-s")),
        ("-Oline", Some("-Oline")),
        ("--output-sync=target", Some("--output-sync=target")),
        ("-j 4", Some("-j 4")),
        ("-l2.5", Some("-l2.5")),
        (
            "--jobserver-auth=fifo:/tmp/archon-jobserver",
            Some("--jobserver-auth=fifo:/tmp/archon-jobserver"),
        ),
        ("--jobserver-auth=3,4", Some("--jobserver-auth=3,4")),
        (
            "-j=4 --jobs=8 -l=2.5 --max-load=3.5",
            Some("-j=4 --jobs=8 -l=2.5 --max-load=3.5"),
        ),
        ("-jabc --jobs=2x -j=1.5 -l2.5.6 --max-load=x -s", Some("-s")),
        ("-j 1.5 -l 2.5.6 -k", Some("-j -l -k")),
        ("--jobserver-auth=3,x --jobserver-fds=3,4,5 -s", Some("-s")),
        ("--jobserver-fds=fifo:/tmp/x=y -s", Some("-s")),
        (
            "--debug=a,b --debug=a1 --output-sync=bogus -Obogus -O -O=",
            Some("--debug=a,b -O -O="),
        ),
        ("-s\\x -k", Some("-k")),
        ("--SERVICE_AUTH=x --SECRET_X=y -s", Some("-s")),
        ("--eval=TOKEN=abc -k", Some("-k")),
        ("--eval TOKEN=abc -s", Some("-s")),
        ("-E TOKEN=abc -s", Some("-s")),
        ("-E -k -s", Some("-s")),
        ("-ETOKEN=abc -kE TOKEN=abc -s", Some("-s")),
        ("--unknown=TOKEN=abc -x=abc --silent", Some("--silent")),
        ("--eval=TOKEN=abc -E TOKEN=abc", None),
        (
            "--jobserver-auth=3,4 --jobserver-fds=5,6 -j4 -j 8 --jobs=2 -l2.5 -l 3 --load-average=4 --debug=a --output-sync=target -Oline -O=target",
            Some(
                "--jobserver-auth=3,4 --jobserver-fds=5,6 -j4 -j 8 --jobs=2 -l2.5 -l 3 --load-average=4 --debug=a --output-sync=target -Oline -O=target",
            ),
        ),
        ("FOO=bar SECRET_X=y SERVICE_AUTH=z", None),
        ("-- FOO=bar SECRET_X=y", None),
        ("FOO=bar\\ -secret -k", Some("-k")),
        ("unexpected words -s", Some("-s")),
        ("", None),
    ] {
        assert_eq!(
            option_flags(OsStr::new(input)),
            expected.map(OsString::from),
            "{input}"
        );
    }
    for key in ["MAKEFLAGS", "MFLAGS", "GNUMAKEFLAGS", "CARGO_MAKEFLAGS"] {
        assert_eq!(
            inherited([(key.into(), "-k FOO=bar SECRET_X=y SERVICE_AUTH=z".into())]),
            vec![(OsString::from(key), OsString::from("-k"))],
        );
        assert!(inherited([(key.into(), "--jobserver-auth=3,4 -- FOO=bar".into())]).is_empty());
    }
}

#[test]
fn exact_environment_preserves_site_bindings_without_ambient_defaults() {
    let site = std::collections::BTreeMap::from([
        ("PATH".into(), "/site/bin".into()),
        ("HOME".into(), "/site/home".into()),
        ("CARGO_HOME".into(), "/site/cargo".into()),
    ]);
    let command = command_with_mode("site-check", Mode::Exact(&site)).0;
    let actual = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.unwrap().to_string_lossy().into_owned(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(actual, site);
}

#[test]
fn only_safe_names_inherit() {
    let vars = [
        "Path",
        "HOME",
        "LANG",
        "TMPDIR",
        "ARCHON_TEST_SECRET",
        "OPENAI_API_KEY",
        "BASH_ENV",
        "LD_PRELOAD",
    ];
    let result = inherited(vars.map(|key| (key.into(), "synthetic".into())));
    assert_eq!(
        result
            .iter()
            .map(|(key, _)| key.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["Path", "HOME", "LANG", "TMPDIR"]
    );
}
