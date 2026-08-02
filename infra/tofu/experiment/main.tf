module "experiment" {
  source = "../modules/aws-experiment"

  run_id                  = var.run_id
  maximum_runtime_seconds = var.maximum_runtime_seconds
  maximum_cost_usd        = var.maximum_cost_usd
}
