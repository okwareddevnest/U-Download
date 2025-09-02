// Sound notification system using Web Audio API with generated tones
class SoundNotifications {
  constructor() {
    this.audioContext = null;
    this.isEnabled = true;
    this.initAudioContext();
  }

  initAudioContext() {
    try {
      const AudioContextClass = window.AudioContext || window.webkitAudioContext;
      if (!AudioContextClass) {
        throw new Error('AudioContext not supported');
      }
      this.audioContext = new AudioContextClass();
    } catch (error) {
      console.warn('Audio context not supported:', error);
      this.isEnabled = false;
    }
  }

  async ensureAudioContext() {
    if (!this.audioContext || !this.isEnabled) return false;
    
    if (this.audioContext.state === 'suspended') {
      try {
        await this.audioContext.resume();
      } catch (error) {
        console.warn('Failed to resume audio context:', error);
        return false;
      }
    }
    return true;
  }

  // Generate a pleasant completion sound (ascending chimes)
  async playDownloadComplete() {
    if (!(await this.ensureAudioContext())) return;

    const frequencies = [523.25, 659.25, 783.99, 1046.50]; // C5, E5, G5, C6
    const duration = 0.15;
    
    for (let i = 0; i < frequencies.length; i++) {
      setTimeout(() => {
        this.playTone(frequencies[i], duration, 0.1, 'sine');
      }, i * 100);
    }
  }

  // Generate an error sound (descending tones)
  async playDownloadError() {
    if (!(await this.ensureAudioContext())) return;

    const frequencies = [400, 300, 200]; // Descending tones
    const duration = 0.3;
    
    for (let i = 0; i < frequencies.length; i++) {
      setTimeout(() => {
        this.playTone(frequencies[i], duration, 0.15, 'square');
      }, i * 150);
    }
  }

  // Generate a start sound (single pleasant tone)
  async playDownloadStart() {
    if (!(await this.ensureAudioContext())) return;
    
    this.playTone(440, 0.2, 0.08, 'sine'); // A4 note
  }

  // Play progress milestone sounds (25%, 50%, 75%)
  async playProgressMilestone(percentage) {
    if (!(await this.ensureAudioContext())) return;

    const toneMap = {
      25: 523.25,  // C5
      50: 659.25,  // E5
      75: 783.99   // G5
    };

    const frequency = toneMap[percentage];
    if (frequency) {
      this.playTone(frequency, 0.1, 0.05, 'sine');
    }
  }

  playTone(frequency, duration, volume = 0.1, waveType = 'sine') {
    if (!this.audioContext || !this.isEnabled) return;

    const oscillator = this.audioContext.createOscillator();
    const gainNode = this.audioContext.createGain();

    oscillator.connect(gainNode);
    gainNode.connect(this.audioContext.destination);

    oscillator.frequency.setValueAtTime(frequency, this.audioContext.currentTime);
    oscillator.type = waveType;

    // Smooth fade in and out to avoid clicks
    gainNode.gain.setValueAtTime(0, this.audioContext.currentTime);
    gainNode.gain.linearRampToValueAtTime(volume, this.audioContext.currentTime + 0.01);
    gainNode.gain.exponentialRampToValueAtTime(0.001, this.audioContext.currentTime + duration);

    oscillator.start(this.audioContext.currentTime);
    oscillator.stop(this.audioContext.currentTime + duration);
  }

  // Enable/disable sounds
  setEnabled(enabled) {
    this.isEnabled = enabled;
    if (enabled && !this.audioContext) {
      this.initAudioContext();
    }
  }

  isAudioEnabled() {
    return this.isEnabled && this.audioContext !== null;
  }
}

// Create singleton instance
const soundNotifications = new SoundNotifications();

// Enable audio context on first user interaction
document.addEventListener('click', () => {
  soundNotifications.ensureAudioContext();
}, { once: true });

export default soundNotifications;