{ stdenv, lib }:

stdenv.mkDerivation {
  pname = "agentos-system-skills";
  version = "0.1.0";

  src = ../../skills;

  installPhase = ''
    mkdir -p $out/skills
    cp -r $src/* $out/skills/
  '';

  meta = with lib; {
    description = "AgentOS system skills collection";
    license = licenses.mit;
  };
}
