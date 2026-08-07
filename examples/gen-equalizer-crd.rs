//! Throwaway generator: prints the EqualizerConfig CRD as YAML so it can be
//! captured to deploy/equalizer/crds.yaml. Removed after generation.
use capacity_admission_webhook::equalizer::crd::EqualizerConfig;
use kube::CustomResourceExt;

fn main() {
    print!(
        "{}",
        serde_yaml::to_string(&EqualizerConfig::crd()).unwrap()
    );
}
