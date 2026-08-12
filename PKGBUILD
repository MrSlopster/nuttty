# Maintainer: MrSlopster <306584837+MrSlopster@users.noreply.github.com>

pkgname=nuttty
pkgver=1.2.1
pkgrel=1
pkgdesc="A btm-inspired TUI dashboard for Network UPS Tools (NUT)"
arch=('x86_64' 'aarch64')
url="https://github.com/MrSlopster/nuttty"
license=('GPL-3.0-or-later')
depends=('gcc-libs')
makedepends=('cargo')
optdepends=('nut: monitor a locally connected UPS')
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
sha512sums=('947285dd85b25a2d52620a38f7998c0ef68aea68f101f925f928ee9ee5c5cbe627a6b0921762ca3f582e32d54c02aa62ad0b48314601dc525f3256eee2a99f14')
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
