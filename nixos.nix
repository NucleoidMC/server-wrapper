self:
{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.nucleoid-server-wrapper;
  settingsFormat = pkgs.formats.toml {};
in {
  options.services.nucleoid-server-wrapper = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = ''
      Enable the Nucleoid server-wrapper.
      '';
    };

    serverPackage = mkOption {
      type = types.package;
      description = ''
      Package providing the server JAR to use.
      '';
    };
    jvmArgs = mkOption {
      type = types.str;
      default = "-Dlog4j2.formatMsgNoLookups=true -Xms1600M -Xmx1600M";
    };
    jvmPackage = mkOption {
      type = types.package;
      description = ''
      Package providing the JVM to use for running the server.
      '';
    };
    destinations = mkOption {
      type = settingsFormat.type;
      description = ''
      List of places to download mods and configurations from.
      '';
    };
  };

  config = {
    systemd.services.nucleoid-server-wrapper = mkIf cfg.enable {
      description = "Nucleoid server-wrapper";
      wantedBy = [ "multi-user.target" ];
      script = ''
      ${}
      '';
    };
  };
}
