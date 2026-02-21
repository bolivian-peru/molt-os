{ stdenv, imagemagick }:

stdenv.mkDerivation {
  pname = "osmoda-plymouth-theme";
  version = "0.1.0";

  src = ./.;

  nativeBuildInputs = [ imagemagick ];

  buildPhase = ''
    # Generate logo PNG: white "osModa" text on transparent background
    convert -size 400x80 xc:transparent \
      -font "DejaVu-Sans" -pointsize 48 -fill white \
      -gravity center -annotate 0 "osModa" \
      logo.png
  '';

  installPhase = ''
    mkdir -p $out/share/plymouth/themes/osmoda
    cp osmoda.plymouth $out/share/plymouth/themes/osmoda/
    cp osmoda.script $out/share/plymouth/themes/osmoda/
    cp logo.png $out/share/plymouth/themes/osmoda/

    # Fix paths in .plymouth to point to Nix store
    substituteInPlace $out/share/plymouth/themes/osmoda/osmoda.plymouth \
      --replace "/share/plymouth/themes/osmoda" "$out/share/plymouth/themes/osmoda"
  '';
}
