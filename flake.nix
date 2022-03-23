{
	inputs = {
		nixCargoIntegration.url = "github:yusdacra/nix-cargo-integration";
	};
	outputs = inputs: inputs.nixCargoIntegration.lib.makeOutputs {
		root = ./.;
	};
}