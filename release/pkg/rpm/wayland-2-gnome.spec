Name:           wayland-2-gnome
Version:        0.1.1
Release:        1%{?dist}
Summary:        Protocol-aware Wayland proxy translating wlr-layer-shell to GNOME Shell overlays
License:        MIT
URL:            https://github.com/leriart/Wayland-2-Gnome
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz
BuildRequires:  cargo
Requires:       glibc

%description
Wayland 2 GNOME acts as a protocol-aware layer between specialized Wayland
applications and the GNOME compositor (Mutter). It enables applications
designed for tiling window managers (like Sway or Hyprland) to run natively
on GNOME by translating incompatible protocols into standard XDG surfaces
in real-time.

%prep
%autosetup

%build
cargo build --release --locked

%check
cargo test --release

%install
install -D -m 0755 target/release/wayland-2-gnome %{buildroot}%{_bindir}/wayland-2-gnome
install -D -m 0644 _build/release/pkg/systemd/wayland-2-gnome.service \
  %{buildroot}%{_userunitdir}/wayland-2-gnome.service
install -D -m 0644 _build/release/autostart/wayland-2-gnome.desktop \
  %{buildroot}%{_sysconfdir}/xdg/autostart/wayland-2-gnome.desktop

%files
%{_bindir}/wayland-2-gnome
%{_userunitdir}/wayland-2-gnome.service
%{_sysconfdir}/xdg/autostart/wayland-2-gnome.desktop
%license LICENSE
%doc README.md

%changelog
* Tue Apr 28 2026 Leriart <leriart@users.noreply.github.com> - 0.1.1-1
- Added --compositor CLI flag for specifying compositor display name

* Tue Apr 28 2026 Leriart <leriart@users.noreply.github.com> - 0.1.0-1
- Initial release
