# osModa Custom Chat UI — The OS Experience
#
# Serves a beautiful dark full-screen chat and proxies all API/WebSocket
# traffic to the OpenClaw gateway running on an internal port.
#
# Flow: Firefox → :18789 (osmoda-ui) → :18790 (OpenClaw gateway)
{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.osmoda;

  # Bundle HTML + server.js into a single Nix store path
  osmodaUi = pkgs.runCommand "osmoda-ui" {} ''
    mkdir -p $out
    cp ${./../../packages/osmoda-ui/index.html} $out/index.html
    cp ${./../../packages/osmoda-ui/server.js} $out/server.js
  '';
in {
  config = mkIf (cfg.enable && cfg.ui.enable) {
    systemd.services.osmoda-ui = {
      description = "osModa Chat UI";
      wantedBy = [ "osmoda-gateway.service" ];
      after = [ "osmoda-gateway.service" ];
      bindsTo = [ "osmoda-gateway.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = "${pkgs.nodejs_22}/bin/node ${osmodaUi}/server.js";
        Restart = "always";
        RestartSec = 3;
      };

      environment = {
        PORT = toString cfg.openclaw.port;
        OPENCLAW_PORT = toString cfg.openclaw.internalPort;
      };
    };
  };
}
