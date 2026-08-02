output "run_id" {
  description = "Run identifier accepted by the module."
  value       = var.run_id
}

output "common_tags" {
  description = "Required tags for every ephemeral resource."
  value       = local.common_tags
}
