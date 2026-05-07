pkgname=t2fanrd-macsmc
pkgver=0.1.0
pkgrel=1
pkgdesc="t2fanrd daemon adapted for macsmc on T2 Macs"
arch=('x86_64')
url="https://github.com/RishonDev/t2fanrd-macsmc"
license=('GPL3')
depends=('glibc' 'systemd')
makedepends=('cargo')
install="$pkgname.install"
source=(
  't2fanrd.service'
)
sha256sums=(
  'SKIP'
)

build() {
  cargo build --release --locked --manifest-path "$startdir/Cargo.toml"
}

package() {
  install -Dm755 "$startdir/target/release/t2fanrd" "$pkgdir/usr/bin/t2fanrd"
  install -Dm644 "$startdir/t2fanrd.service" "$pkgdir/usr/lib/systemd/system/t2fanrd.service"
  install -Dm644 "$startdir/README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 "$startdir/LICENCE" "$pkgdir/usr/share/licenses/$pkgname/LICENCE"
}
