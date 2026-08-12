# Maintainer: MrSlopster <306584837+MrSlopster@users.noreply.github.com>

pkgname=nuttty
pkgver=1.2.0
pkgrel=1
pkgdesc="A btm-inspired TUI dashboard for Network UPS Tools (NUT)"
arch=('x86_64' 'aarch64')
url="https://github.com/MrSlopster/nuttty"
license=('GPL-3.0-or-later')
depends=('gcc-libs')
makedepends=('cargo')
optdepends=('nut: monitor a locally connected UPS')
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
sha512sums=('086cf5c2e3f13d007789b0df3f327308770b6fc131610900e5c4b5041cbd8671a7025bf52280e241a2bf451f699ba1a6883ee1032de4db3103ad46f594825a65')
options=(!debug !lto)

prepare() {
  cd "$pkgname-$pkgver"
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "$pkgname-$pkgver"
  cargo build --release --locked --offline
}

check() {
  cd "$pkgname-$pkgver"
  cargo test --release --locked --offline
}

package() {
  cd "$pkgname-$pkgver"
  install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
  install -Dm644 README.md -t "$pkgdir/usr/share/doc/$pkgname"
  install -Dm644 LICENSE -t "$pkgdir/usr/share/licenses/$pkgname"
}
