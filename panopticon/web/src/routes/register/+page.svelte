<script lang="ts">
	import { goto } from '$app/navigation';

	let email = $state('');
	let password = $state('');
	let confirmPassword = $state('');
	let error: string | null = $state(null);
	let loading = $state(false);

	async function handleRegister(e: Event) {
		e.preventDefault();
		error = null;

		if (password !== confirmPassword) {
			error = 'Passwords do not match';
			return;
		}

		loading = true;

		try {
			const res = await fetch('/api/auth/register', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ email, password })
			});

			const data = await res.json();

			if (!res.ok) {
				error = data.error || 'Registration failed';
				return;
			}

			goto('/');
		} catch {
			error = 'Network error. Please try again.';
		} finally {
			loading = false;
		}
	}
</script>

<svelte:head>
	<title>Sign Up — Panopticon</title>
</svelte:head>

<main class="flex flex-1 items-center justify-center p-6">
	<div class="w-full max-w-sm space-y-6">
		<div class="text-center">
			<h1 class="h2">Panopticon</h1>
			<p class="mt-2 text-sm text-surface-400">Create a new account</p>
		</div>

		<div class="card preset-filled-surface-900 space-y-5 p-6">
			{#if error}
				<div class="rounded-md bg-error-500/10 px-4 py-3 text-sm text-error-400">
					{error}
				</div>
			{/if}

			<form onsubmit={handleRegister} class="space-y-4">
				<label class="label space-y-2">
					<span class="label-text">Email</span>
					<input
						type="email"
						bind:value={email}
						required
						class="input preset-filled-surface-800 border border-surface-700 px-4 py-2.5"
						placeholder="you@example.com"
					/>
				</label>

				<label class="label space-y-2">
					<span class="label-text">Password</span>
					<input
						type="password"
						bind:value={password}
						required
						minlength="8"
						class="input preset-filled-surface-800 border border-surface-700 px-4 py-2.5"
						placeholder="At least 8 characters"
					/>
				</label>

				<label class="label space-y-2">
					<span class="label-text">Confirm Password</span>
					<input
						type="password"
						bind:value={confirmPassword}
						required
						minlength="8"
						class="input preset-filled-surface-800 border border-surface-700 px-4 py-2.5"
						placeholder="••••••••"
					/>
				</label>

				<button type="submit" class="btn btn-base preset-filled-primary-500 w-full" disabled={loading}>
					{loading ? 'Creating account...' : 'Create Account'}
				</button>
			</form>
		</div>

		<p class="text-center text-sm text-surface-500">
			Already have an account? <a href="/login" class="anchor">Sign in</a>
		</p>
	</div>
</main>
