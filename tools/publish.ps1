# Publish deckd to GitHub, but only when the signed-in gh account is the owner's.
# Run from the repo root after reviewing `git log`:   powershell -File tools\publish.ps1
$owner = "Huss3A"
$repo  = "deckd"

if ((gh api user --jq .login) -ne $owner) { "Not signed in as $owner (run: gh auth login)"; exit 1 }

git branch -M main

# Create the repo when it does not exist on GitHub (a leftover `origin` pointing at a deleted repo is replaced).
gh repo view "$owner/$repo" *> $null
if ($LASTEXITCODE -ne 0) {
    if (git remote | Select-String -Quiet "^origin$") { git remote remove origin }
    gh repo create "$owner/$repo" --public --source . --remote origin --description "Rust replacement for Ulanzi Studio (Ulanzi D200H)"
    if ($LASTEXITCODE -ne 0) { "Could not create $owner/$repo"; exit 1 }
}
git push -u origin main
