## How it works

<div class="run-flow">
  <div class="run-step">
    <div class="run-step-num">1</div>
    <div class="run-step-title">Parse</div>
    <div class="run-step-body">Recipe text becomes a typed AST. Schema-checked against the type catalog before anything runs.</div>
  </div>
  <div class="run-arrow">→</div>
  <div class="run-step">
    <div class="run-step-num">2</div>
    <div class="run-step-title">Fetch</div>
    <div class="run-step-body">Engine runs the HTTP graph or drives a headless browser. Pagination and rate-limits are applied.</div>
  </div>
  <div class="run-arrow">→</div>
  <div class="run-step">
    <div class="run-step-num">3</div>
    <div class="run-step-title">Extract</div>
    <div class="run-step-body">Walks responses, evaluates path expressions, builds typed records.</div>
  </div>
  <div class="run-arrow">→</div>
  <div class="run-step">
    <div class="run-step-num">4</div>
    <div class="run-step-title">Report</div>
    <div class="run-step-body">Returns a snapshot plus a diagnostic report. Both can be archived to disk for replay.</div>
  </div>
</div>

## Examples

<RecipeCarousel />

## Features
