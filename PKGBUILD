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
sha512sums=('de3dfd2cd195d547099374cfc4b4adee60595ff36931a10d68b95fd6f73be93daf4fac9dc284933cd3495c6caa512e971abad8c2b0a90bdcb33243b99fde4c92')
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
