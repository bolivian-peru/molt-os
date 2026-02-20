{ stdenv, imagemagick }:

stdenv.mkDerivation {
  pname = "agentos-plymouth-theme";
  version = "0.1.0";

  src = ./.;

  nativeBuildInputs = [ imagemagick ];

  buildPhase = ''
    # Generate a simple logo PNG: white "AgentOS" text on transparent background
    # Using ImageMagick to create the wordmark — no external assets needed
    convert -size 400x80 xc:transparent \
      -font "DejaVu-Sans" -pointsize 48 -fill white \
      -gravity center -annotate 0 "AgentOS" \
      logo.png
  '';

  installPhase = ''
    mkdir -p $out/share/plymouth/themes/agentos
    cp agentos.plymouth $out/share/plymouth/themes/agentos/
    cp agentos.script $out/share/plymouth/themes/agentos/
    cp logo.png $out/share/plymouth/themes/agentos/

    # Fix paths in .plymouth to point to Nix store
    substituteInPlace $out/share/plymouth/themes/agentos/agentos.plymouth \
      --replace "/share/plymouth/themes/agentos" "$out/share/plymouth/themes/agentos"
  '';
}
