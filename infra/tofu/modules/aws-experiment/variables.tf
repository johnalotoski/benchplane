variable "run_id" {
  description = "Globally unique Benchplane run identifier used for tags and state isolation."
  type        = string

  validation {
    condition     = length(var.run_id) >= 8
    error_message = "run_id must contain at least eight characters."
  }
}

variable "maximum_runtime_seconds" {
  description = "Hard maximum run lifetime."
  type        = number
  default     = 3600

  validation {
    condition     = var.maximum_runtime_seconds > 0 && var.maximum_runtime_seconds <= 86400
    error_message = "maximum_runtime_seconds must be between 1 and 86400."
  }
}

variable "maximum_cost_usd" {
  description = "Experiment cost ceiling used by the caller and future guards."
  type        = number

  validation {
    condition     = var.maximum_cost_usd > 0
    error_message = "maximum_cost_usd must be positive."
  }
}
