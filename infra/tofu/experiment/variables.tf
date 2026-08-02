variable "aws_region" {
  type        = string
  description = "AWS region selected by the resolved experiment plan."
}

variable "run_id" {
  type        = string
  description = "Unique Benchplane run identifier."
}

variable "maximum_runtime_seconds" {
  type        = number
  description = "Hard maximum run lifetime."
  default     = 3600
}

variable "maximum_cost_usd" {
  type        = number
  description = "Maximum permitted experiment cost."
}
