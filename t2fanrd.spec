Name:           t2fanrd-macsmc
Version:        0.1.0
Release:        1%{?dist}
Summary:        t2fanrd daemon adapted for macsmc on T2 Macs
License:        GPL-3.0-or-later
URL:            https://github.com/RishonDev/t2fanrd-macsmc
Source0:        %{name}-%{version}.tar.gz
Source1:        t2fanrd.service

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  systemd-rpm-macros
Requires:       systemd

%description
A refreshed t2fanrd daemon adapted for macsmc on T2 Macs.

%prep
%autosetup -n t2fanrd-%{version}

%build
cargo build --release --locked

%install
install -Dm755 target/release/t2fanrd %{buildroot}%{_bindir}/t2fanrd
install -Dm644 %{SOURCE1} %{buildroot}%{_unitdir}/t2fanrd.service
install -Dm644 README.md %{buildroot}%{_docdir}/%{name}/README.md
install -Dm644 LICENCE %{buildroot}%{_licensedir}/%{name}/LICENCE

%post
%systemd_post t2fanrd.service

%preun
%systemd_preun t2fanrd.service

%postun
%systemd_postun_with_restart t2fanrd.service

%files
%license %{_licensedir}/%{name}/LICENCE
%doc %{_docdir}/%{name}/README.md
%{_bindir}/t2fanrd
%{_unitdir}/t2fanrd.service

%changelog
* Fri May 08 2026 Rishon Dev <rishon@example.com> - 0.1.0-1
- Initial Fedora package for macsmc-adapted t2fanrd
