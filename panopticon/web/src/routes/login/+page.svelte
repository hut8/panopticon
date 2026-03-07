<script lang="ts">
	import { goto } from '$app/navigation';

	let email = $state('');
	let password = $state('');
	let error: string | null = $state(null);
	let loading = $state(false);

	// NFC state
	let nfcSupported = $state(typeof window !== 'undefined' && 'NDEFReader' in window);
	let nfcScanning = $state(false);
	let nfcStatus: string | null = $state(null);

	async function handleLogin(e: Event) {
		e.preventDefault();
		error = null;
		loading = true;

		try {
			const res = await fetch('/api/auth/login', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ email, password })
			});

			const data = await res.json();

			if (!res.ok) {
				error = data.error || 'Login failed';
				return;
			}

			goto('/');
		} catch {
			error = 'Network error. Please try again.';
		} finally {
			loading = false;
		}
	}

	async function handleNfcLogin() {
		if (!nfcSupported || nfcScanning) return;

		error = null;
		nfcStatus = 'Hold your NFC tag near the device...';
		nfcScanning = true;

		try {
			const ndef = new (window as any).NDEFReader();
			await ndef.scan();

			ndef.addEventListener('reading', async ({ serialNumber }: { serialNumber: string }) => {
				nfcStatus = 'Tag detected, signing in...';

				try {
					const res = await fetch('/api/auth/nfc/login', {
						method: 'POST',
						headers: { 'Content-Type': 'application/json' },
						body: JSON.stringify({ serial: serialNumber })
					});

					const data = await res.json();

					if (!res.ok) {
						error = data.error || 'NFC login failed';
						nfcStatus = null;
						nfcScanning = false;
						return;
					}

					goto('/');
				} catch {
					error = 'Network error. Please try again.';
					nfcStatus = null;
					nfcScanning = false;
				}
			});

			ndef.addEventListener('readingerror', () => {
				error = 'Could not read NFC tag. Try again.';
				nfcStatus = null;
				nfcScanning = false;
			});
		} catch (e: any) {
			if (e.name === 'NotAllowedError') {
				error = 'NFC permission denied. Allow NFC access and try again.';
			} else if (e.name === 'NotSupportedError') {
				error = 'NFC is not available on this device.';
				nfcSupported = false;
			} else {
				error = 'Failed to start NFC scan.';
			}
			nfcStatus = null;
			nfcScanning = false;
		}
	}

	function cancelNfcScan() {
		nfcScanning = false;
		nfcStatus = null;
	}
</script>

<svelte:head>
	<title>Sign In — Panopticon</title>
</svelte:head>

<main class="flex flex-1 items-center justify-center p-6">
	<div class="w-full max-w-sm space-y-6">
		<div class="text-center">
			<h1 class="h2">Panopticon</h1>
			<p class="mt-2 text-sm text-surface-400">Sign in to your account</p>
		</div>

		<div class="card preset-filled-surface-900 space-y-5 p-6">
			{#if error}
				<div class="rounded-md bg-error-500/10 px-4 py-3 text-sm text-error-400">
					{error}
				</div>
			{/if}

			{#if nfcSupported}
				{#if nfcScanning}
					<div class="space-y-4 text-center">
						<div class="mx-auto flex h-16 w-16 items-center justify-center rounded-full bg-primary-500/15">
							<svg class="h-8 w-8 text-primary-400 animate-pulse" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
								<path stroke-linecap="round" stroke-linejoin="round" d="M8.288 15.038a5.25 5.25 0 017.424 0M5.106 11.856c3.807-3.808 9.98-3.808 13.788 0M1.924 8.674c5.565-5.565 14.587-5.565 20.152 0M12.53 18.22l-.53.53-.53-.53a.75.75 0 011.06 0z" />
							</svg>
						</div>
						{#if nfcStatus}
							<p class="text-sm text-surface-300">{nfcStatus}</p>
						{/if}
						<button
							type="button"
							class="btn btn-sm preset-outlined-surface-500"
							onclick={cancelNfcScan}
						>
							Cancel
						</button>
					</div>
				{:else}
					<button
						type="button"
						class="btn btn-base preset-outlined-primary-500 w-full flex items-center justify-center gap-2"
						onclick={handleNfcLogin}
					>
						<svg class="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
							<path stroke-linecap="round" stroke-linejoin="round" d="M8.288 15.038a5.25 5.25 0 017.424 0M5.106 11.856c3.807-3.808 9.98-3.808 13.788 0M1.924 8.674c5.565-5.565 14.587-5.565 20.152 0M12.53 18.22l-.53.53-.53-.53a.75.75 0 011.06 0z" />
						</svg>
						Tap NFC to Sign In
					</button>
				{/if}

				<div class="flex items-center gap-3">
					<div class="h-px flex-1 bg-surface-700"></div>
					<span class="text-xs text-surface-500">or use email</span>
					<div class="h-px flex-1 bg-surface-700"></div>
				</div>
			{/if}

			<form onsubmit={handleLogin} class="space-y-4">
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
						class="input preset-filled-surface-800 border border-surface-700 px-4 py-2.5"
						placeholder="••••••••"
					/>
				</label>

				<button type="submit" class="btn btn-base preset-filled-primary-500 w-full" disabled={loading}>
					{loading ? 'Signing in...' : 'Sign In'}
				</button>
			</form>

			<div class="text-center text-sm">
				<a href="/forgot-password" class="anchor">Forgot your password?</a>
			</div>
		</div>

		<p class="text-center text-sm text-surface-500">
			No account? <a href="/register" class="anchor">Create one</a>
		</p>
	</div>
</main>
