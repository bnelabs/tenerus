%global debug_package %{nil}
%global __strip /bin/true
%global __brp_strip %{nil}
%global __brp_strip_comment_note %{nil}
%global __brp_strip_static_archive %{nil}
%global __brp_check_rpaths %{nil}

Name:           tenerus
Version:        %{_version}
Release:        1%{?dist}
Summary:        Local-first desktop market workspace for educational scenario analysis and AI research

License:        MIT
URL:            https://github.com/bnelabs/tenerus
AutoReqProv:    no

Requires:       curl
Requires:       mesa-libGL
Requires:       libX11
Requires:       libxkbcommon
Requires:       libwayland-client

%description
Tenerus is an independent native desktop market workspace for studying
historical price distributions, running local analogue replays, and auditing
AI-assisted market commentary against fixed baselines. Provided solely for
educational and research purposes.

%install
rm -rf %{buildroot}
install -Dm755 "%{_bin}" "%{buildroot}%{_bindir}/tenerus"
install -Dm644 "%{_root}/packaging/linux/tenerus.desktop" "%{buildroot}%{_datadir}/applications/tenerus.desktop"
install -Dm644 "%{_root}/assets/tenerus-icon-256.png" "%{buildroot}%{_datadir}/icons/hicolor/256x256/apps/tenerus.png"
install -Dm644 "%{_root}/LICENSE" "%{buildroot}%{_licensedir}/%{name}/LICENSE"
install -Dm644 "%{_root}/README.md" "%{buildroot}%{_docdir}/%{name}/README.md"
install -Dm644 "%{_root}/CREDITS.md" "%{buildroot}%{_docdir}/%{name}/CREDITS.md"
install -Dm644 "%{_root}/CHANGELOG.md" "%{buildroot}%{_docdir}/%{name}/CHANGELOG.md"

%files
%{_bindir}/tenerus
%{_datadir}/applications/tenerus.desktop
%{_datadir}/icons/hicolor/256x256/apps/tenerus.png
%license %{_licensedir}/%{name}/LICENSE
%doc %{_docdir}/%{name}/README.md
%doc %{_docdir}/%{name}/CREDITS.md
%doc %{_docdir}/%{name}/CHANGELOG.md
