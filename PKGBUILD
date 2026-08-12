# Maintainer: MrSlopster <306584837+MrSlopster@users.noreply.github.com>

pkgname=nuttty
pkgver=1.0.0
pkgrel=1
pkgdesc="A btm-inspired TUI dashboard for Network UPS Tools (NUT)"
arch=('x86_64' 'aarch64')
url="https://github.com/MrSlopster/nuttty"
license=('GPL-3.0-or-later')
depends=('gcc-libs')
makedepends=('cargo')
optdepends=('nut: monitor a locally connected UPS')
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
sha512sums=('3c1d3d99c789c6960002aad3334a7c994bd6b746a64ab878014503257afa5029760d0f014a172f4aa42dfd1c4266b566e3efea4a54edf7a9f9131e3bbf9719b9')
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
