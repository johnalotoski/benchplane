locals {
  common_tags = {
    "benchplane:managed" = "true"
    "benchplane:run-id"  = var.run_id
  }
}

# Intentionally resource-free during the local milestone.
# The first AWS implementation should add one bounded experiment node and the
# minimum supporting resources, rather than prematurely decomposing every AWS
# object into a separate module.
